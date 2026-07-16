use arrow::array::*;
use arrow::buffer::{BooleanBuffer, NullBuffer};
use arrow::datatypes::*;
use arrow::record_batch::RecordBatch;

use std::collections::HashMap;

use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;

use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct PlottableSeries<'a> {
    /// The zero-allocation zipped coordinate iterator
    pub coords: Box<dyn Iterator<Item = (f64, f64)> + 'a>,
    pub y_min: f64,
    pub y_max: f64,
}

pub struct TableStore {
    // We use RwLock so multiple threads can read simultaneously,
    // but only one thread can mutate at a time.
    columns: Arc<RwLock<Vec<GenericColumn>>>,
    pub signalmap: Arc<RwLock<HashMap<String, usize>>>,
    ready_state: Arc<(Mutex<bool>, Condvar)>, // Ready is set once schema is complete. Once set, cannot be unset.
}

impl TableStore {
    /// Creates a new TableStore wrapping the columns.
    pub fn new() -> Self {
        Self {
            columns: Arc::new(RwLock::new(Vec::new())),
            ready_state: Arc::new((Mutex::new(false), Condvar::new())),
            signalmap: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_ready(&self) {
        let (lock, cvar) = &*self.ready_state;
        let mut ready = lock.lock().expect("Mutex poisoned");
        *ready = true;

        // Wake up every thread currently parked on wait_until_ready()
        cvar.notify_all();
    }

    pub fn wait_until_ready(&self) {
        let (lock, cvar) = &*self.ready_state;
        let mut ready = lock.lock().expect("Mutex poisoned");

        // As long as the flag is false, put the thread to sleep efficiently
        while !*ready {
            ready = cvar.wait(ready).expect("Condvar poisoned");
        }
    }

    /// Cheaply clones the atomic reference pointer to the columns
    /// so it can be safely passed to another thread.
    pub fn clone_ref(&self) -> Self {
        Self {
            columns: Arc::clone(&self.columns),
            ready_state: Arc::clone(&self.ready_state),
            signalmap: Arc::clone(&self.signalmap),
        }
    }

    /// Read columns (use this *after* wait_until_ready)
    pub fn read_columns(&self) -> RwLockReadGuard<'_, Vec<GenericColumn>> {
        self.columns.read().expect("RwLock poisoned")
    }

    /// Write columns (used by the initializing thread)
    pub fn write_columns(&self) -> RwLockWriteGuard<'_, Vec<GenericColumn>> {
        self.columns.write().expect("RwLock poisoned")
    }

    pub fn find_n_back_by_period(&self, period: f64) -> usize {
        let cols = self.columns.read().expect("lock poisoned");
        let time_col = match &cols[0] {
            GenericColumn::F64(c) => &c.values,
            _ => panic!("Time column must be F64"),
        };

        if time_col.is_empty() {
            return 0;
        }

        let time_len = time_col.len();

        let current_time = time_col[time_len - 1];
        let target_time = current_time - period;

        // Binary search handles irregular/dynamic step sizes
        match time_col.binary_search_by(|probe| probe.partial_cmp(&target_time).unwrap()) {
            Ok(idx) => time_len - idx,  // Exact match found
            Err(idx) => time_len - idx, // Closest index following the target time
        } // TODO - add gating to make sure time column never decreases. otherwise this gonna break
    }

    /// O(log n) Lookback: Finds the index that represents exactly `period` seconds ago.
    /// Ideal for real-time plotting windows.
    pub fn find_index_back_by_period(&self, period: f64) -> usize {
        let cols = self.columns.read().expect("lock poisoned");
        let time_col = match &cols[0] {
            GenericColumn::F64(c) => &c.values,
            _ => panic!("Time column must be F64"),
        };

        if time_col.is_empty() {
            return 0;
        }

        let current_time = time_col[time_col.len() - 1];
        let target_time = current_time - period;

        // Binary search handles irregular/dynamic step sizes
        match time_col.binary_search_by(|probe| probe.partial_cmp(&target_time).unwrap()) {
            Ok(idx) => idx,  // Exact match found
            Err(idx) => idx, // Closest index following the target time
        } // TODO - add gating to make sure time column never decreases. otherwise this gonna break
    }

    pub fn get_plot_series<'a>(
        &self,
        cols: &'a [GenericColumn], // Pass the read guard's columns slice
        target_col_idx: usize,
        start_idx: usize,
        end_idx: usize,
    ) -> Option<PlottableSeries<'a>> {
        // 1. Extract the time slice ONCE
        let (time_vals, time_valids) = match &cols[0] {
            GenericColumn::F64(c) => c.get_slice(start_idx, end_idx)?,
            _ => panic!("Time column must be F64"),
        };

        // 2. Extract the target column iterator
        let target_col = cols.get(target_col_idx)?;

        // We will do a single-pass run to find the min and max of the visible window
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;

        // Materialize target iterator to calculate bounds
        let target_iter = target_col.iter_as_f64(start_idx, end_idx)?;

        // To calculate bounds without taking ownership of the iterator,
        // we can peek at the data, but since we want to avoid double-iteration overhead,
        // we map the iterator to track limits as values flow through it:
        let bounded_iter = target_iter
            .enumerate()
            .filter_map(move |(i, (val, valid))| {
                // Ensure both the time index and signal value are valid
                if valid && time_valids[i] {
                    let t = time_vals[i];
                    return Some((t, val));
                }
                None
            });

        // Let's perform a fast bounding check on the target values
        // (This replaces your previous separate bounds calculation loop)
        let mut cached_points = Vec::with_capacity(end_idx - start_idx);
        for (t, y) in bounded_iter {
            if y < y_min {
                y_min = y;
            }
            if y > y_max {
                y_max = y;
            }
            cached_points.push((t, y));
        }

        if cached_points.is_empty() {
            return None;
        }

        Some(PlottableSeries {
            coords: Box::new(cached_points.into_iter()),
            y_min,
            y_max,
        })
    }

    /// Looks up a column by index and finds the most recent non-null value, cast to f64.
    pub fn get_latest_valid_value_by_index(&self, col_idx: usize) -> Option<f64> {
        let cols = self.columns.read().expect("lock poisoned");
        cols.get(col_idx)?.last_valid_value_as_f64()
    }
}

pub struct Column<T> {
    pub values: Vec<T>,
    pub valid: Vec<bool>, // true = valid, false = null
}

impl<T: Default + Copy> Column<T> {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            valid: Vec::new(),
        }
    }

    pub fn push(&mut self, value: Option<T>) {
        match value {
            Some(v) => {
                self.values.push(v);
                self.valid.push(true);
            }
            None => {
                self.values.push(T::default());
                self.valid.push(false);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns a slice of the last `n` elements.
    /// If the column has fewer than `n` elements, returns all elements.
    pub fn last_n_values(&self, n: usize) -> &[T] {
        let len = self.values.len();
        let start = len.saturating_sub(n);
        &self.values[start..len]
    }

    /// Returns a slice of the last `n` validity flags.
    pub fn last_n_validity(&self, n: usize) -> &[bool] {
        let len = self.valid.len();
        let start = len.saturating_sub(n);
        &self.valid[start..len]
    }

    pub fn start_to_n_values(&self, start: usize, n: usize) -> &[T] {
        &self.values[start..n]
    }

    pub fn start_to_n_validity(&self, start: usize, n: usize) -> &[bool] {
        &self.valid[start..n]
    }

    pub fn get_slice(&self, start: usize, end: usize) -> Option<(&[T], &[bool])> {
        if start <= end && end <= self.values.len() {
            Some((&self.values[start..end], &self.valid[start..end]))
        } else {
            None
        }
    }

    /// Searches backwards from the end of the column to find the most recent
    /// valid (non-null) value.
    pub fn last_valid_value(&self) -> Option<T> {
        // Walk backwards through the validity flags
        let valid_idx = self
            .valid
            .iter()
            .enumerate()
            .rev()
            .find(|&(_, is_valid)| *is_valid)
            .map(|(idx, _)| idx)?;

        Some(self.values[valid_idx])
    }
}

pub enum GenericColumn {
    Bool(Column<bool>),
    F64(Column<f64>),
    F32(Column<f32>),
    //    F16(Column<f16>),
    I8(Column<i8>),
    I16(Column<i16>),
    I32(Column<i32>),
    I64(Column<i64>),
}

impl GenericColumn {
    pub fn push_null(&mut self) {
        match self {
            GenericColumn::Bool(c) => c.push(None),
            GenericColumn::F64(c) => c.push(None),
            GenericColumn::F32(c) => c.push(None),
            GenericColumn::I8(c) => c.push(None),
            GenericColumn::I16(c) => c.push(None),
            GenericColumn::I32(c) => c.push(None),
            GenericColumn::I64(c) => c.push(None),
        }
    }

    pub fn data_type(&self) -> DataType {
        match self {
            GenericColumn::Bool(_) => DataType::Boolean,
            GenericColumn::I8(_) => DataType::Int8,
            GenericColumn::I16(_) => DataType::Int16,
            GenericColumn::I32(_) => DataType::Int32,
            GenericColumn::I64(_) => DataType::Int64,
            GenericColumn::F32(_) => DataType::Float32,
            GenericColumn::F64(_) => DataType::Float64,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            GenericColumn::Bool(c) => c.len(),
            GenericColumn::I8(c) => c.len(),
            GenericColumn::I16(c) => c.len(),
            GenericColumn::I32(c) => c.len(),
            GenericColumn::I64(c) => c.len(),
            GenericColumn::F32(c) => c.len(),
            GenericColumn::F64(c) => c.len(),
        }
    }

    pub fn as_f64_slice(&self, n: usize) -> Option<&[f64]> {
        if let GenericColumn::F64(c) = self {
            Some(c.last_n_values(n))
        } else {
            None
        }
    }

    pub fn finish(self) -> Arc<dyn Array> {
        match self {
            GenericColumn::Bool(c) => {
                // BooleanArray is unique because values are stored as bits, not bytes
                let val_buf = BooleanBuffer::from(c.values);
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(BooleanArray::new(val_buf, Some(null_buf)))
            }
            GenericColumn::I8(c) => {
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(Int8Array::new(c.values.into(), Some(null_buf)))
            }
            GenericColumn::I16(c) => {
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(Int16Array::new(c.values.into(), Some(null_buf)))
            }
            GenericColumn::I32(c) => {
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(Int32Array::new(c.values.into(), Some(null_buf)))
            }
            GenericColumn::I64(c) => {
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(Int64Array::new(c.values.into(), Some(null_buf)))
            }
            GenericColumn::F32(c) => {
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(Float32Array::new(c.values.into(), Some(null_buf)))
            }
            GenericColumn::F64(c) => {
                let null_buf = NullBuffer::from(c.valid);
                Arc::new(Float64Array::new(c.values.into(), Some(null_buf)))
            }
        }
    }

    pub fn clone_to_array(&self) -> Arc<dyn Array> {
        match self {
            GenericColumn::Bool(c) => {
                let val_buf = BooleanBuffer::from(c.values.clone());
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(BooleanArray::new(val_buf, Some(null_buf)))
            }
            GenericColumn::I8(c) => {
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(Int8Array::new(c.values.clone().into(), Some(null_buf)))
            }
            GenericColumn::I16(c) => {
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(Int16Array::new(c.values.clone().into(), Some(null_buf)))
            }
            GenericColumn::I32(c) => {
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(Int32Array::new(c.values.clone().into(), Some(null_buf)))
            }
            GenericColumn::I64(c) => {
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(Int64Array::new(c.values.clone().into(), Some(null_buf)))
            }
            GenericColumn::F32(c) => {
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(Float32Array::new(c.values.clone().into(), Some(null_buf)))
            }
            GenericColumn::F64(c) => {
                let null_buf = NullBuffer::from(c.valid.clone());
                Arc::new(Float64Array::new(c.values.clone().into(), Some(null_buf)))
            }
        }
    }

    pub fn iter_as_f64<'a>(
        &'a self,
        start: usize,
        end: usize,
    ) -> Option<Box<dyn Iterator<Item = (f64, bool)> + 'a>> {
        match self {
            GenericColumn::Bool(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter()
                        .zip(valids)
                        .map(|(&v, &valid)| (if v { 1.0 } else { 0.0 }, valid)),
                ))
            }
            GenericColumn::I8(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter()
                        .zip(valids)
                        .map(|(&v, &valid)| (v as f64, valid)),
                ))
            }
            GenericColumn::I16(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter()
                        .zip(valids)
                        .map(|(&v, &valid)| (v as f64, valid)),
                ))
            }
            GenericColumn::I32(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter()
                        .zip(valids)
                        .map(|(&v, &valid)| (v as f64, valid)),
                ))
            }
            GenericColumn::I64(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter()
                        .zip(valids)
                        .map(|(&v, &valid)| (v as f64, valid)),
                ))
            }
            GenericColumn::F32(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter()
                        .zip(valids)
                        .map(|(&v, &valid)| (v as f64, valid)),
                ))
            }
            GenericColumn::F64(c) => {
                let (vals, valids) = c.get_slice(start, end)?;
                Some(Box::new(
                    vals.iter().zip(valids).map(|(&v, &valid)| (v, valid)),
                ))
            }
        }
    }

    /// Finds the most recent valid (non-null) value in this column, cast to an f64.
    /// Returns `None` if the column is entirely empty or contains only nulls.
    pub fn last_valid_value_as_f64(&self) -> Option<f64> {
        match self {
            GenericColumn::Bool(c) => c.last_valid_value().map(|v| if v { 1.0 } else { 0.0 }),
            GenericColumn::I8(c) => c.last_valid_value().map(|v| v as f64),
            GenericColumn::I16(c) => c.last_valid_value().map(|v| v as f64),
            GenericColumn::I32(c) => c.last_valid_value().map(|v| v as f64),
            GenericColumn::I64(c) => c.last_valid_value().map(|v| v as f64),
            GenericColumn::F32(c) => c.last_valid_value().map(|v| v as f64),
            GenericColumn::F64(c) => c.last_valid_value(), // No cast needed
        }
    }
}

pub fn finish_record_batch(columns: Vec<GenericColumn>, schema: Arc<Schema>) -> RecordBatch {
    assert!(!columns.is_empty());

    let row_count = columns[0].len();

    for c in &columns {
        assert_eq!(c.len(), row_count, "column length mismatch");
    }

    let arrays: Vec<Arc<dyn Array>> = columns.into_iter().map(|c| c.finish()).collect();

    RecordBatch::try_new(schema, arrays).expect("failed to create RecordBatch")
}

pub fn write_record_batch_to_parquet(
    batch: &RecordBatch,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(path)?;

    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))?;

    writer.write(batch)?;
    writer.close()?;

    Ok(())
}
