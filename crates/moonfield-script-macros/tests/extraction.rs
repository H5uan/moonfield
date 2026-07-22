//! Exercises the `#[script_function]`-generated argument extraction path
//! (`ScriptFunction::call`) with real `HostValue` arguments. The unit tests
//! in `src/lib.rs` only cover TypeScript signature generation; these cover
//! the runtime marshaling — signed/wide integer narrowing, `f32`, `Vec<u8>`,
//! numeric `Vec<T>`, and the `Option<T>` variants.

use moonfield_script::script::{HostValue, ScriptFunction, TypedArrayValue};
use moonfield_script_macros::script_function;

#[script_function]
fn passthrough_i32(v: i32) -> Result<i32, String> {
    Ok(v)
}

#[script_function]
fn passthrough_f32(v: f32) -> Result<f64, String> {
    Ok(v as f64)
}

#[script_function]
fn passthrough_u64(v: u64) -> Result<f64, String> {
    Ok(v as f64)
}

#[script_function]
fn passthrough_i64(v: i64) -> Result<f64, String> {
    Ok(v as f64)
}

#[script_function]
fn byte_sum(bytes: Vec<u8>) -> Result<i32, String> {
    Ok(bytes.iter().map(|&b| b as i32).sum())
}

#[script_function]
fn vec_sum(values: Vec<f32>) -> Result<f64, String> {
    Ok(values.iter().map(|&v| v as f64).sum())
}

#[script_function]
fn option_or_default(v: Option<i32>) -> Result<i32, String> {
    Ok(v.unwrap_or(-1))
}

#[script_function]
fn optional_bytes_len(bytes: Option<Vec<u8>>) -> Result<i32, String> {
    Ok(bytes.map(|b| b.len() as i32).unwrap_or(-1))
}

#[script_function]
fn narrow_u8(v: u8) -> Result<i32, String> {
    Ok(v as i32)
}

#[script_function]
fn describe(tag: String, count: u32, ratio: f64, flag: bool) -> Result<String, String> {
    Ok(format!("{tag}:{count}:{ratio}:{flag}"))
}

#[script_function]
fn optional_u32(v: Option<u32>) -> Result<i32, String> {
    Ok(v.map(|n| n as i32).unwrap_or(-1))
}

/// `i32` previously extracted via `as_u32()`, rejecting all negatives.
#[test]
fn i32_accepts_negative_values() {
    let v = passthrough_i32_Fn::call(&[HostValue::Number(-42.0)]).expect("call");
    assert_eq!(v.as_f64(), Some(-42.0));
}

/// `f32` previously fell into the `as_u32()` catch-all, truncating 1.5 to 1.
#[test]
fn f32_keeps_fractional_part() {
    let v = passthrough_f32_Fn::call(&[HostValue::Number(1.5)]).expect("call");
    assert_eq!(v.as_f64(), Some(1.5));
}

/// `u64`/`i64` previously extracted via `as_u32()`, failing past `u32::MAX`.
#[test]
fn wide_integers_accept_large_values() {
    // Beyond u32::MAX but exactly representable in f64 (< 2^53).
    let big = 5_000_000_000.0;
    let v = passthrough_u64_Fn::call(&[HostValue::Number(big)]).expect("call");
    assert_eq!(v.as_f64(), Some(big));

    let v = passthrough_i64_Fn::call(&[HostValue::Number(-big)]).expect("call");
    assert_eq!(v.as_f64(), Some(-big));
}

/// `Vec<u8>` extracts from the bytes representations (both the owned
/// ArrayBuffer and Uint8Array marshaling go through `as_bytes`).
#[test]
fn vec_u8_extracts_from_bytes() {
    let v = byte_sum_Fn::call(&[HostValue::ArrayBuffer(vec![1, 2, 3])]).expect("call");
    assert_eq!(v.as_f64(), Some(6.0));

    let v = byte_sum_Fn::call(&[HostValue::TypedArray(TypedArrayValue::Uint8(vec![10, 20]))])
        .expect("call");
    assert_eq!(v.as_f64(), Some(30.0));

    // A non-bytes argument is a type mismatch, not a panic.
    let err = byte_sum_Fn::call(&[HostValue::Number(1.0)]).expect_err("number is not bytes");
    assert!(err.contains("expected Vec<u8>"), "unexpected error: {err}");
}

/// Numeric vectors extract from the plain array representation, converting
/// each element; one bad element fails the whole extraction.
#[test]
fn numeric_vec_extracts_from_array() {
    let v = vec_sum_Fn::call(&[HostValue::Array(vec![
        HostValue::Number(1.5),
        HostValue::Number(2.5),
    ])])
    .expect("call");
    assert_eq!(v.as_f64(), Some(4.0));

    let err = vec_sum_Fn::call(&[HostValue::Array(vec![
        HostValue::Number(1.0),
        HostValue::Bool(true),
    ])])
    .expect_err("non-numeric element must fail");
    assert!(err.contains("expected Vec<f32>"), "unexpected error: {err}");
}

/// `Option<i32>` previously extracted via `as_u32()`, turning `Some(-5)`
/// into `None`.
#[test]
fn option_i32_preserves_negative_some() {
    let v = option_or_default_Fn::call(&[HostValue::Number(-5.0)]).expect("call");
    assert_eq!(v.as_f64(), Some(-5.0));

    // Missing and mistyped arguments still map to `None`.
    let v = option_or_default_Fn::call(&[]).expect("call");
    assert_eq!(v.as_f64(), Some(-1.0));
    let v = option_or_default_Fn::call(&[HostValue::Bool(true)]).expect("call");
    assert_eq!(v.as_f64(), Some(-1.0));
}

/// `Option<Vec<u8>>` mirrors the plain `Vec<u8>` extraction.
#[test]
fn option_vec_u8_extracts_from_bytes() {
    let v = optional_bytes_len_Fn::call(&[HostValue::ArrayBuffer(vec![1, 2, 3])]).expect("call");
    assert_eq!(v.as_f64(), Some(3.0));

    let v = optional_bytes_len_Fn::call(&[]).expect("call");
    assert_eq!(v.as_f64(), Some(-1.0));
}

/// Narrowing must fail loudly: out-of-range and fractional values produce a
/// clear error instead of silently truncating or wrapping.
#[test]
fn out_of_range_narrowing_errors() {
    let err = narrow_u8_Fn::call(&[HostValue::Number(300.0)]).expect_err("300 overflows u8");
    assert!(
        err.contains("does not fit in u8"),
        "unexpected error: {err}"
    );

    let err = narrow_u8_Fn::call(&[HostValue::Number(-1.0)]).expect_err("negative overflows u8");
    assert!(
        err.contains("does not fit in u8"),
        "unexpected error: {err}"
    );

    let err = narrow_u8_Fn::call(&[HostValue::Number(1.5)])
        .expect_err("fractional must not silently truncate");
    assert!(
        err.contains("does not fit in u8"),
        "unexpected error: {err}"
    );

    let err = passthrough_i32_Fn::call(&[HostValue::Number(3e9)]).expect_err("3e9 overflows i32");
    assert!(
        err.contains("does not fit in i32"),
        "unexpected error: {err}"
    );

    // A non-number argument is a plain type mismatch.
    let err = narrow_u8_Fn::call(&[HostValue::Bool(true)]).expect_err("bool is not a number");
    assert!(err.contains("expected u8"), "unexpected error: {err}");
}

/// The pre-existing arms keep their exact behavior.
#[test]
fn existing_arms_still_extract() {
    let v = describe_Fn::call(&[
        HostValue::String("fps".to_string()),
        HostValue::Number(60.0),
        HostValue::Number(0.5),
        HostValue::Bool(true),
    ])
    .expect("call");
    assert_eq!(v.as_str(), Some("fps:60:0.5:true"));

    // Option<u32> keeps its semantics: missing -> None, present -> Some.
    let v = optional_u32_Fn::call(&[]).expect("call");
    assert_eq!(v.as_f64(), Some(-1.0));
    let v = optional_u32_Fn::call(&[HostValue::Number(8.0)]).expect("call");
    assert_eq!(v.as_f64(), Some(8.0));
}
