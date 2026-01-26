// DBC Parsing
use dbc_rs::Dbc;
use std::fs;

// Arrow IP elements
use arrow::datatypes::{DataType, Field, Schema};
use std::sync::Arc;

// Custom data storage helpers
pub mod store;
use store::{Column, GenericColumn};

// Used for type decisions only
trait FloatExt {
    fn is_nearly(&self, target: f64) -> bool;
}

impl FloatExt for f64 {
    fn is_nearly(&self, target: f64) -> bool {
        // Use a slightly larger margin than f64::EPSILON
        // if you expect multiple cumulative calculations.
        (self - target).abs() < f64::EPSILON
    }
}

struct Decode {
    dbc: Dbc,

}

impl Decode
