use arrow::array::*;
use arrow::buffer::{BooleanBuffer, NullBuffer};
use arrow::datatypes::*;
use arrow::record_batch::RecordBatch;
use dbc_rs::Message;

use std::collections::HashMap;

use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;
use std::fs::File;

use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const PERIOD_BIN_PRECISION: f64 = 5.0;
pub const META_CHUNK_LENGTH: usize = 100;

#[derive(Clone)]
pub struct StridedPoints<'a> {
    col: &'a GenericColumn,
    time: &'a [f64],
    time_valid: &'a [bool],
    base: usize, // start_idx
    pos: usize,  // next bucket start
    n: usize,    // window length
    stride: usize,
}

impl<'a> Iterator for StridedPoints<'a> {
    type Item = (f64, f64);
    fn next(&mut self) -> Option<(f64, f64)> {
        while self.pos < self.n {
            let lo = self.pos;
            let hi = (lo + self.stride).min(self.n);
            self.pos = hi;
            for i in lo..hi {
                if self.time_valid[i] {
                    if let Some((v, true)) = self.col.value_at_f64(self.base + i) {
                        return Some((self.time[i], v));
                    }
                }
            }
        }
        None
    }
}

pub struct Segmenter<'a> {
    inner: std::iter::Peekable<StridedPoints<'a>>,
    gap: f64,
}

impl<'a> Segmenter<'a> {
    pub fn new(points: StridedPoints<'a>, gap: f64) -> Self {
        Self {
            inner: points.peekable(),
            gap,
        }
    }

    pub fn next_segment(&mut self) -> Option<Segment<'_, 'a>> {
        self.inner.peek()?;
        Some(Segment {
            parent: self,
            last_t: None,
        })
    }
}

pub struct Segment<'s, 'a> {
    parent: &'s mut Segmenter<'a>,
    last_t: Option<f64>,
}

impl Iterator for Segment<'_, '_> {
    type Item = (f64, f64);
    fn next(&mut self) -> Option<(f64, f64)> {
        let &(t, _) = self.parent.inner.peek()?;
        if let Some(lt) = self.last_t {
            if t - lt > self.parent.gap {
                return None; // gap: leave the point for the next segment
            }
        }
        self.last_t = Some(t);
        self.parent.inner.next()
    }
}

#[derive(Default, Clone)]
struct SignalProgress {
    rows_done: usize,    // rows of the column already consumed
    last_t: Option<f64>, // time of the last valid sample seen
}

#[derive(Default)]
struct PeriodHistCache {
    /// hists[signal] = (period, count), kept sorted by period
    hists: Vec<Vec<(f64, usize)>>,
    progress: Vec<SignalProgress>,
}

impl PeriodHistCache {
    fn clear(&mut self) {
        self.hists.clear();
        self.progress.clear();
    }

    fn bump(hist: &mut Vec<(f64, usize)>, period: f64) {
        // Snap to the nearest bin. Same arithmetic every time, so equal bins
        // produce bit-identical floats and compare cleanly.
        let snapped = (period / PERIOD_BIN_PRECISION).round() * PERIOD_BIN_PRECISION;
        match hist.binary_search_by(|(p, _)| p.total_cmp(&snapped)) {
            Ok(i) => hist[i].1 += 1,
            Err(i) => hist.insert(i, (snapped, 1)),
        }
    }

    /// Consumes any rows added since the last call for this signal.
    fn update(&mut self, cols: &[GenericColumn], idx: usize) -> Option<()> {
        let target = cols.get(idx)?;
        let GenericColumn::F64(time) = cols.first()? else {
            panic!("Time column must be F64");
        };
        let n = time.len();

        if self.hists.len() <= idx {
            self.hists.resize_with(idx + 1, Vec::new);
            self.progress.resize_with(idx + 1, SignalProgress::default);
        }
        let prog = &mut self.progress[idx];
        let hist = &mut self.hists[idx];

        if prog.rows_done >= n {
            return Some(()); // already current
        }

        let (t_vals, t_valid) = time.get_slice(prog.rows_done, n)?;
        let sig = target.iter_as_f64(prog.rows_done, n)?;

        for (i, (_, valid)) in sig.enumerate() {
            if valid && t_valid[i] {
                let t = t_vals[i];
                if let Some(prev) = prog.last_t {
                    Self::bump(hist, t - prev);
                }
                prog.last_t = Some(t);
            }
        }
        prog.rows_done = n;
        Some(())
    }
}

#[derive(Debug, Clone)]
pub struct MessageSend {
    pub name: String,
    pub content: Vec<(Box<str>, f64)>,
    pub repeat: usize,
}

pub struct PlottableSeries<'a> {
    pub points: StridedPoints<'a>,
    pub gap: f64, // effective split threshold for the sampled points
    pub y_min: f64,
    pub y_max: f64,
    pub downsample: usize,
}

pub struct TableStore {
    // We use RwLock so multiple threads can read simultaneously,
    // but only one thread can mutate at a time.
    pub columns: Arc<RwLock<Vec<GenericColumn>>>,
    pub signalmap: Arc<RwLock<HashMap<String, usize>>>,
    pub nodes: Arc<RwLock<HashMap<String, Vec<Message>>>>,
    pub sends: Arc<RwLock<Vec<MessageSend>>>,
    ready_state: Arc<(Mutex<bool>, Condvar)>, // Ready is set once schema is complete. Once set, cannot be unset.
    converted: Arc<Mutex<usize>>,
    period_cache: Arc<Mutex<PeriodHistCache>>,
}

impl TableStore {
    /// Creates a new TableStore wrapping the columns.
    pub fn new() -> Self {
        Self {
            columns: Arc::new(RwLock::new(Vec::new())),
            ready_state: Arc::new((Mutex::new(false), Condvar::new())),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            signalmap: Arc::new(RwLock::new(HashMap::new())),
            sends: Arc::new(RwLock::new(Vec::new())),
            converted: Arc::new(Mutex::new(0)),
            period_cache: Arc::new(Mutex::new(PeriodHistCache::default())),
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
            nodes: Arc::clone(&self.nodes),
            signalmap: Arc::clone(&self.signalmap),
            sends: Arc::clone(&self.sends),
            converted: Arc::clone(&self.converted),
            period_cache: Arc::clone(&self.period_cache),
        }
    }

    /// Read columns (use this *after* wait_until_ready)
    pub fn read_columns(&self) -> RwLockReadGuard<'_, Vec<GenericColumn>> {
        if let Some(read_guard) = self.columns.try_read_for(Duration::from_secs(1)) {
            return read_guard;
        } else {
            println!("Timeout waiting for read lock");
            panic!();
        }
    }

    /// Write columns (used by the initializing thread)
    pub fn write_columns(&self) -> RwLockWriteGuard<'_, Vec<GenericColumn>> {
        if let Some(write_guard) = self.columns.try_write_for(Duration::from_secs(1)) {
            return write_guard;
        } else {
            println!("Timeout waiting for write lock");
            panic!();
        }
    }

    /// Returns the first index in the time column whose value is >= `target`.
    /// - Returns `len` if all samples are earlier than `target` (i.e. "past the end").
    /// - Returns 0 if the table is empty or `target` precedes the first sample.
    /// Assumes the time column is sorted ascending (see TODO below).
    pub fn find_index_by_value(&self, target: f64) -> usize {
        let cols = self.read_columns();
        Self::index_by_value_locked(&cols, target)
    }

    /// Same search, for callers that already hold a read guard
    /// (avoids re-locking and keeps the result consistent with later slicing).
    pub fn index_by_value_locked(cols: &[GenericColumn], target: f64) -> usize {
        let time_col = match cols.first() {
            Some(GenericColumn::F64(c)) => &c.values,
            Some(_) => panic!("Time column must be F64"),
            None => return 0, // schema not populated yet
        };

        // partition_point is a lower-bound binary search: O(log n), no unwrap,
        // and with duplicate timestamps it deterministically returns the FIRST one
        // (binary_search_by returns an arbitrary match among duplicates).
        time_col.partition_point(|&t| t < target)
    }

    pub fn find_index_back_by_period(&self, period: f64) -> usize {
        let cols = self.read_columns();
        let Some(GenericColumn::F64(c)) = cols.first() else {
            return 0;
        };
        let Some(&current_time) = c.values.last() else {
            return 0;
        };
        Self::index_by_value_locked(&cols, current_time - period)
    }

    pub fn get_plot_series<'a>(
        &self,
        cols: &'a [GenericColumn],
        target_col_idx: usize,
        start_idx: usize,
        end_idx: usize,
        separate_threshold: f64,
        max_points: usize,
    ) -> Option<PlottableSeries<'a>> {
        let (time, time_valid) = match &cols[0] {
            GenericColumn::F64(c) => c.get_slice(start_idx, end_idx)?,
            _ => panic!("Time column must be F64"),
        };
        let col = cols.get(target_col_idx)?;

        let n = end_idx - start_idx;
        let max_points = max_points.max(1);
        let mut stride = 1usize;
        while n.div_ceil(stride) > max_points {
            stride *= 2;
        }

        let points = StridedPoints {
            col,
            time,
            time_valid,
            base: start_idx,
            pos: 0,
            n,
            stride,
        };

        let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut any = false;

        let block_meta: Option<&[Meta]> = match col {
            GenericColumn::F64(c) => Some(&c.meta_cache),
            GenericColumn::F32(c) => Some(&c.meta_cache),
            GenericColumn::I8(c) => Some(&c.meta_cache),
            GenericColumn::I16(c) => Some(&c.meta_cache),
            GenericColumn::I32(c) => Some(&c.meta_cache),
            GenericColumn::I64(c) => Some(&c.meta_cache),
            GenericColumn::Bool(_) => None,
        };

        if let Some(metas) = block_meta.filter(|_| n > 10 * META_CHUNK_LENGTH) {
            let range_end = end_idx.saturating_sub(1);
            for meta in metas {
                if meta.index_start > range_end {
                    break;
                }
                if meta.index_end < start_idx {
                    continue;
                }
                if meta.min != f64::INFINITY {
                    any = true;
                    y_min = y_min.min(meta.min);
                }
                if meta.max != f64::NEG_INFINITY {
                    any = true;
                    y_max = y_max.max(meta.max);
                }
            }
        } else {
            for (_, y) in points.clone() {
                // Only ref clone
                any = true;
                y_min = y_min.min(y);
                y_max = y_max.max(y);
            }
        }

        if !any {
            return None;
        }

        Some(PlottableSeries {
            points,
            // Sampled points are `stride` rows apart, so scale the threshold
            gap: separate_threshold * stride as f64,
            y_min,
            y_max,
            downsample: stride,
        })
    }

    /// Looks up a column by index and finds the most recent non-null value, cast to f64.
    pub fn get_latest_valid_value_by_index(&self, col_idx: usize) -> Option<f64> {
        let cols = self.read_columns();
        cols.get(col_idx)?.last_valid_value_as_f64()
    }

    /// Copies the next `limit` unconverted rows (or all remaining if `limit` is None)
    /// and advances the cursor. Returns None if there's nothing to take.
    fn take_unconverted(&self, limit: Option<usize>) -> Option<Vec<GenericColumn>> {
        let cols = self.read_columns(); // lock order: columns...
        let mut cursor = self.converted.lock().expect("Mutex poisoned"); // ...then cursor

        let nrows = cols.first().map_or(0, |c| c.len());
        let available = nrows.saturating_sub(*cursor);
        let take = match limit {
            Some(n) if available >= n => n,
            Some(_) => return None, // not enough for a full batch yet
            None => available,
        };
        if take == 0 {
            return None;
        }

        let start = *cursor;
        let end = start + take;
        let owned = cols.iter().map(|c| c.clone_range(start, end)).collect();
        *cursor = end;
        Some(owned)
    } // both locks released here, before Arrow conversion

    /// Returns the next full batch of exactly `n` fresh rows, or None if fewer are available.
    pub fn next_record_batch(&self, n: usize, schema: Arc<Schema>) -> Option<RecordBatch> {
        let owned = self.take_unconverted(Some(n))?;
        Some(finish_record_batch(owned, schema))
    }

    /// Returns whatever unconverted rows remain (for shutdown).
    pub fn flush_record_batch(&self, schema: Arc<Schema>) -> Option<RecordBatch> {
        let owned = self.take_unconverted(None)?;
        Some(finish_record_batch(owned, schema))
    }

    /// Removes the oldest `n` rows (clamped to table length). Returns rows removed.
    /// Shifts the cursor down by the same amount.
    pub fn purge_oldest(&self, n: usize) -> usize {
        let mut cols = self.write_columns();
        let mut cursor = self.converted.lock().expect("Mutex poisoned");
        Self::purge_locked(&mut cols, &mut cursor, n)
    }

    /// Removes only rows that have already been converted.
    pub fn purge_converted(&self) -> usize {
        let mut cols = self.write_columns();
        let mut cursor = self.converted.lock().expect("Mutex poisoned");
        let n = *cursor;
        Self::purge_locked(&mut cols, &mut cursor, n)
    }

    fn purge_locked(cols: &mut [GenericColumn], cursor: &mut usize, n: usize) -> usize {
        let nrows = cols.first().map_or(0, |c| c.len());
        let k = n.min(nrows);
        if k == 0 {
            return 0;
        }
        for c in cols.iter_mut() {
            c.drop_front(k);
        }
        *cursor = cursor.saturating_sub(k);
        k
    }

    pub fn get_period_hist_by_index(
        &self,
        cols: &[GenericColumn],
        index: usize,
    ) -> Option<Vec<(f64, usize)>> {
        let mut cache = self.period_cache.lock().expect("Mutex poisoned");
        cache.update(cols, index)?;
        Some(cache.hists[index].clone())
    }

    pub fn recompute_metas(cols: &mut [GenericColumn]) {
        for col in cols {
            col.recompute_metas();
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub index_start: usize,
    pub index_end: usize,
    pub max: f64,
    pub avg: f64,
    pub min: f64,
    pub num_points: usize,
}

impl Meta {
    fn new(index_start: usize) -> Self {
        Self {
            index_start,
            index_end: index_start,
            max: f64::NEG_INFINITY,
            avg: 0.0,
            min: f64::INFINITY,
            num_points: 0,
        }
    }

    fn record_valid(&mut self, index: usize, val: f64) {
        self.index_end = index;
        if val.is_nan() {
            return;
        }
        self.num_points += 1;
        let n = self.num_points as f64;
        if self.num_points == 1 {
            self.min = val;
            self.max = val;
            self.avg = val;
        } else {
            if val < self.min {
                self.min = val;
            }
            if val > self.max {
                self.max = val;
            }
            self.avg += (val - self.avg) / n;
        }
    }

    fn record_null(&mut self, index: usize) {
        self.index_end = index;
    }
}

pub struct Column<T> {
    pub values: Vec<T>,
    pub valid: Vec<bool>, // true = valid, false = null
    pub meta_cache: Vec<Meta>,
    last_valid: Option<usize>,
}

impl<T: Default + Copy + IntoF64> Column<T> {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            valid: Vec::new(),
            meta_cache: Vec::new(),
            last_valid: None,
        }
    }

    pub fn push(&mut self, value: Option<T>) {
        let idx = self.values.len();
        let block_start = (idx / META_CHUNK_LENGTH) * META_CHUNK_LENGTH;

        let needs_new_block = match self.meta_cache.last() {
            None => true,
            Some(last) => idx > last.index_end && (idx - last.index_start) >= META_CHUNK_LENGTH,
        };
        if needs_new_block {
            self.meta_cache.push(Meta::new(block_start));
        }

        match value {
            Some(v) => {
                self.values.push(v);
                self.valid.push(true);
                self.last_valid = Some(idx);
                if let Some(meta) = self.meta_cache.last_mut() {
                    meta.record_valid(idx, v.into_f64());
                }
            }
            None => {
                self.values.push(T::default());
                self.valid.push(false);
                if let Some(meta) = self.meta_cache.last_mut() {
                    meta.record_null(idx);
                }
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
    pub fn last_valid_index_scan(&self) -> Option<usize> {
        let last_valid: Option<usize> = self
            .valid
            .iter()
            .enumerate()
            .rev()
            .find(|&(_, is_valid)| *is_valid)
            .map(|(idx, _)| idx);

        last_valid
    }

    pub fn last_valid_value(&self) -> Option<T> {
        self.last_valid.map(|i| self.values[i])
    }

    /// Owned copy of rows [start..end).
    pub fn clone_range(&self, start: usize, end: usize) -> Column<T> {
        Column {
            values: self.values[start..end].to_vec(),
            valid: self.valid[start..end].to_vec(),
            meta_cache: self.meta_cache[start..end].to_vec(),
            last_valid: self.last_valid_index_scan(),
        }
    }

    pub fn drop_front(&mut self, n: usize) {
        if n == 0 || self.values.is_empty() {
            return;
        }

        let n = n.min(self.values.len());
        self.values.drain(..n);
        self.valid.drain(..n);
        self.last_valid = self.last_valid.and_then(|i| i.checked_sub(n));

        // Drop blocks completely before the new front (`index_end < n`).
        self.meta_cache.retain(|m| m.index_end >= n);

        // Shift remaining block indices left by `n`.
        for meta in &mut self.meta_cache {
            meta.index_start = meta.index_start.saturating_sub(n);
            meta.index_end = meta.index_end.saturating_sub(n);
        }

        // If the first remaining block now starts at 0 and still spans elements,
        // recompute its stats so dropped front points do not skew min/avg/max.
        if let Some(first) = self.meta_cache.first_mut() {
            if first.index_start == 0 && !self.values.is_empty() {
                let end_idx = first.index_end.min(self.values.len() - 1);
                let mut rebuilt = Meta::new(0);
                for i in 0..=end_idx {
                    if self.valid[i] {
                        rebuilt.record_valid(i, self.values[i].into_f64());
                    } else {
                        rebuilt.record_null(i);
                    }
                }
                *first = rebuilt;
            }
        }
    }

    pub fn recompute_metas(&mut self) {
        self.meta_cache.clear();
        self.last_valid = None;

        if self.values.is_empty() {
            return;
        }

        let len = self.values.len();
        let mut block_start = 0usize;

        while block_start < len {
            let block_end = (block_start + META_CHUNK_LENGTH).min(len) - 1;
            let mut meta = Meta::new(block_start);

            for idx in block_start..=block_end {
                if *self.valid.get(idx).unwrap_or(&false) {
                    self.last_valid = Some(idx);
                    meta.record_valid(idx, self.values[idx].into_f64());
                } else {
                    meta.record_null(idx);
                }
            }

            self.meta_cache.push(meta);
            block_start += META_CHUNK_LENGTH;
        }
    }
}

pub trait IntoF64 {
    fn into_f64(self) -> f64;
}

impl IntoF64 for f64 {
    fn into_f64(self) -> f64 {
        self
    }
}

impl IntoF64 for f32 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i8 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i16 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i32 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i64 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for bool {
    fn into_f64(self) -> f64 {
        (self as u8) as f64
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

    pub fn value_at_f64(&self, i: usize) -> Option<(f64, bool)> {
        Some(match self {
            GenericColumn::Bool(c) => (if *c.values.get(i)? { 1.0 } else { 0.0 }, c.valid[i]),
            GenericColumn::I8(c) => (*c.values.get(i)? as f64, c.valid[i]),
            GenericColumn::I16(c) => (*c.values.get(i)? as f64, c.valid[i]),
            GenericColumn::I32(c) => (*c.values.get(i)? as f64, c.valid[i]),
            GenericColumn::I64(c) => (*c.values.get(i)? as f64, c.valid[i]),
            GenericColumn::F32(c) => (*c.values.get(i)? as f64, c.valid[i]),
            GenericColumn::F64(c) => (*c.values.get(i)?, c.valid[i]),
        })
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

    pub fn clone_range(&self, start: usize, end: usize) -> GenericColumn {
        match self {
            GenericColumn::Bool(c) => GenericColumn::Bool(c.clone_range(start, end)),
            GenericColumn::I8(c) => GenericColumn::I8(c.clone_range(start, end)),
            GenericColumn::I16(c) => GenericColumn::I16(c.clone_range(start, end)),
            GenericColumn::I32(c) => GenericColumn::I32(c.clone_range(start, end)),
            GenericColumn::I64(c) => GenericColumn::I64(c.clone_range(start, end)),
            GenericColumn::F32(c) => GenericColumn::F32(c.clone_range(start, end)),
            GenericColumn::F64(c) => GenericColumn::F64(c.clone_range(start, end)),
        }
    }

    pub fn drop_front(&mut self, n: usize) {
        match self {
            GenericColumn::Bool(c) => c.drop_front(n),
            GenericColumn::I8(c) => c.drop_front(n),
            GenericColumn::I16(c) => c.drop_front(n),
            GenericColumn::I32(c) => c.drop_front(n),
            GenericColumn::I64(c) => c.drop_front(n),
            GenericColumn::F32(c) => c.drop_front(n),
            GenericColumn::F64(c) => c.drop_front(n),
        }
    }

    pub fn recompute_metas(&mut self) {
        match self {
            GenericColumn::Bool(c) => c.recompute_metas(),
            GenericColumn::I8(c) => c.recompute_metas(),
            GenericColumn::I16(c) => c.recompute_metas(),
            GenericColumn::I32(c) => c.recompute_metas(),
            GenericColumn::I64(c) => c.recompute_metas(),
            GenericColumn::F32(c) => c.recompute_metas(),
            GenericColumn::F64(c) => c.recompute_metas(),
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

impl GenericColumn {
    /// An empty column of the variant matching an Arrow type, if this store supports it.
    fn empty_for(data_type: &DataType) -> Option<GenericColumn> {
        Some(match data_type {
            DataType::Boolean => GenericColumn::Bool(Column::new()),
            DataType::Int8 => GenericColumn::I8(Column::new()),
            DataType::Int16 => GenericColumn::I16(Column::new()),
            DataType::Int32 => GenericColumn::I32(Column::new()),
            DataType::Int64 => GenericColumn::I64(Column::new()),
            DataType::Float32 => GenericColumn::F32(Column::new()),
            DataType::Float64 => GenericColumn::F64(Column::new()),
            _ => return None,
        })
    }

    /// Appends every row of an Arrow array, keeping its null mask.
    /// Errors if the array's type doesn't match this column's variant.
    fn append_array(&mut self, array: &ArrayRef) -> Result<(), String> {
        fn extend_primitive<A: ArrowPrimitiveType>(
            col: &mut Column<A::Native>,
            arr: &PrimitiveArray<A>,
        ) {
            let base = col.values.len();
            col.values.extend_from_slice(arr.values());

            // Newest valid row within the appended range, as an absolute index
            let new_last = match arr.nulls() {
                Some(nulls) => {
                    col.valid.extend(nulls.iter());
                    // Scan backward over only the new flags
                    col.valid[base..].iter().rposition(|&v| v).map(|i| base + i)
                }
                None => {
                    col.valid.resize(base + arr.len(), true);
                    (arr.len() > 0).then(|| base + arr.len() - 1)
                }
            };

            // Only overwrite if this batch contained a valid row; otherwise the
            // previous batch's last valid index is still the newest
            if new_last.is_some() {
                col.last_valid = new_last;
            }
        }

        let mismatch = || format!("array type {:?} doesn't match column", array.data_type());
        match self {
            GenericColumn::Bool(c) => {
                let arr = array.as_boolean_opt().ok_or_else(mismatch)?;
                for i in 0..arr.len() {
                    c.push(arr.is_valid(i).then(|| arr.value(i)));
                }
            }
            GenericColumn::I8(c) => extend_primitive(
                c,
                array.as_primitive_opt::<Int8Type>().ok_or_else(mismatch)?,
            ),
            GenericColumn::I16(c) => extend_primitive(
                c,
                array.as_primitive_opt::<Int16Type>().ok_or_else(mismatch)?,
            ),
            GenericColumn::I32(c) => extend_primitive(
                c,
                array.as_primitive_opt::<Int32Type>().ok_or_else(mismatch)?,
            ),
            GenericColumn::I64(c) => extend_primitive(
                c,
                array.as_primitive_opt::<Int64Type>().ok_or_else(mismatch)?,
            ),
            GenericColumn::F32(c) => extend_primitive(
                c,
                array
                    .as_primitive_opt::<Float32Type>()
                    .ok_or_else(mismatch)?,
            ),
            GenericColumn::F64(c) => extend_primitive(
                c,
                array
                    .as_primitive_opt::<Float64Type>()
                    .ok_or_else(mismatch)?,
            ),
        }
        Ok(())
    }
}

/// Reads an entire parquet file into the store, replacing whatever columns it held.
pub fn load_parquet(
    file: File,
    cols: &mut RwLockWriteGuard<'_, Vec<GenericColumn>>,
) -> Result<Arc<Schema>, Box<dyn std::error::Error>> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();
    let reader = builder.build()?;

    let mut new_cols = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        new_cols.push(GenericColumn::empty_for(field.data_type()).ok_or_else(|| {
            format!(
                "unsupported parquet column type {:?} for '{}'",
                field.data_type(),
                field.name()
            )
        })?);
    }
    if !matches!(new_cols.first(), Some(GenericColumn::F64(_))) {
        return Err("first parquet column must be the f64 time column".into());
    }

    for batch in reader {
        let batch = batch?;
        for ((col, array), field) in new_cols
            .iter_mut()
            .zip(batch.columns())
            .zip(schema.fields())
        {
            col.append_array(array)
                .map_err(|e| format!("column '{}': {}", field.name(), e))?;
        }
    }

    **cols = new_cols;

    TableStore::recompute_metas(cols);
    Ok(schema)
}
