use std::sync::Arc;

use arrow::array::*;
use arrow::buffer::{BooleanBuffer, NullBuffer};
use arrow::datatypes::*;
use arrow::record_batch::RecordBatch;

use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;

pub struct Column<T> {
    values: Vec<T>,
    valid: Vec<bool>, // true = valid, false = null
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
