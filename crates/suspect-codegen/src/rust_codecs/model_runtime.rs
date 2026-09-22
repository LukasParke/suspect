use crate::{JsonValue, JsonNonNullValue, Nullable, Presence};
use crate::validation::{ValidationFinding, ValidationOutcome, ValidationSession};

/// A transport, source validation, evaluation, or native conversion failure.
#[derive(Debug)]
pub enum CodecError {
    /// Exact JSON syntax, duplicate-key, Unicode or resource error.
    Json(crate::JsonError),
    /// A complete schema evaluation rejected the value.
    Invalid(Vec<ValidationFinding>),
    /// Evaluation could not complete within its finite policy.
    EvaluationFailure(ValidationFinding),
    /// A native representation or its conversion budget rejected the value.
    Conversion(ValidationFinding),
}
impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for CodecError {}
impl From<crate::JsonError> for CodecError { fn from(e: crate::JsonError) -> Self { Self::Json(e) } }
struct Context {
    validation: ValidationSession,
    remaining: usize,
    document: &'static str,
    pointer: &'static str,
    instance_path: String,
}
impl Context {
    fn new(document: &'static str, pointer: &'static str) -> Self { Self { validation: ValidationSession::new(), remaining: CONVERSION_STEPS, document, pointer, instance_path: String::new() } }
    fn error(&self, message: &str) -> CodecError { CodecError::Conversion(ValidationFinding { document: self.document.into(), pointer: self.pointer.into(), instance_path: self.instance_path.clone(), message: message.into() }) }
    fn spend(&mut self, n: usize) -> Result<(), CodecError> { self.remaining = self.remaining.checked_sub(n).ok_or_else(|| self.error("native conversion work limit exceeded"))?; Ok(()) }
    fn step(&mut self, depth: usize) -> Result<(), CodecError> { if depth > CONVERSION_DEPTH { return Err(self.error("native conversion depth limit exceeded")); } self.spend(1) }
    fn check(&mut self, root: usize, value: &JsonValue) -> Result<(), CodecError> {
        match self.validation.validate_at(root,value,&self.instance_path) { ValidationOutcome::Valid => Ok(()), ValidationOutcome::Invalid(e) => Err(CodecError::Invalid(e)), ValidationOutcome::EvaluationFailure(e) => Err(CodecError::EvaluationFailure(e)) }
    }
    fn member(&mut self, root: usize, value: &JsonValue) -> Result<bool, CodecError> {
        match self.validation.validate_at(root,value,&self.instance_path) { ValidationOutcome::Valid => Ok(true), ValidationOutcome::Invalid(_) => Ok(false), ValidationOutcome::EvaluationFailure(e) => Err(CodecError::EvaluationFailure(e)) }
    }
    fn literal(&mut self, root: usize, value: &JsonValue, literal: &JsonValue) -> Result<bool, CodecError> {
        self.spend(1)?;
        self.validation.equal_at(root,value,literal,&self.instance_path).map_err(CodecError::EvaluationFailure)
    }
    fn source<T>(&mut self, document: &'static str, pointer: &'static str, convert: impl FnOnce(&mut Self) -> Result<T,CodecError>) -> Result<T,CodecError> {
        let previous = (self.document,self.pointer);
        self.document=document; self.pointer=pointer;
        let result=convert(self);
        (self.document,self.pointer)=previous;
        result
    }
    fn at<T>(&mut self, token: &str, convert: impl FnOnce(&mut Self) -> Result<T,CodecError>) -> Result<T,CodecError> {
        self.spend(token.len().saturating_add(1))?;
        let length=self.instance_path.len();
        self.instance_path.push('/');
        for ch in token.chars() { match ch { '~'=>self.instance_path.push_str("~0"), '/'=>self.instance_path.push_str("~1"), ch=>self.instance_path.push(ch) } }
        let result=convert(self);
        self.instance_path.truncate(length);
        result
    }
    fn at_index<T>(&mut self, index: usize, convert: impl FnOnce(&mut Self) -> Result<T,CodecError>) -> Result<T,CodecError> {
        use std::fmt::Write as _;
        self.spend(index.checked_ilog10().unwrap_or(0) as usize + 2)?;
        let length=self.instance_path.len();
        write!(&mut self.instance_path,"/{index}").expect("writing to String");
        let result=convert(self);
        self.instance_path.truncate(length);
        result
    }
}
fn copy_json(v: &JsonValue, cx: &mut Context, d: usize) -> Result<JsonValue,CodecError> {
    cx.step(d)?;
    Ok(match v {
        Nullable::Null => Nullable::Null,
        Nullable::Value(v) => Nullable::Value(match v {
            JsonNonNullValue::Bool(v) => JsonNonNullValue::Bool(*v),
            JsonNonNullValue::String(v) => { cx.spend(v.len())?; JsonNonNullValue::String(v.clone()) },
            JsonNonNullValue::Number(v) => { cx.spend(v.as_str().len())?; JsonNonNullValue::Number(v.clone()) },
            JsonNonNullValue::Array(v) => JsonNonNullValue::Array(v.iter().enumerate().map(|(i,v)| cx.at_index(i,|cx| copy_json(v,cx,d+1))).collect::<Result<_,_>>()?),
            JsonNonNullValue::Object(v) => JsonNonNullValue::Object(v.iter().map(|(k,v)| { cx.spend(k.len())?; Ok((k.clone(),cx.at(k,|cx| copy_json(v,cx,d+1))?)) }).collect::<Result<_,CodecError>>()?),
        }),
    })
}
trait Scalar: Sized {
    fn from_json(v: JsonValue, cx: &mut Context) -> Result<Self,CodecError>;
    fn to_json(&self, cx: &mut Context, d: usize) -> Result<JsonValue,CodecError>;
}
impl Scalar for String {
    fn from_json(v: JsonValue,cx: &mut Context)->Result<Self,CodecError>{ match v { Nullable::Value(JsonNonNullValue::String(v))=>Ok(v),_=>Err(cx.error("expected string")) } }
    fn to_json(&self,cx:&mut Context,_d:usize)->Result<JsonValue,CodecError>{cx.spend(self.len())?;Ok(Nullable::Value(JsonNonNullValue::String(self.clone())))}
}
impl Scalar for bool {
    fn from_json(v:JsonValue,cx:&mut Context)->Result<Self,CodecError>{match v {Nullable::Value(JsonNonNullValue::Bool(v))=>Ok(v),_=>Err(cx.error("expected boolean"))}}
    fn to_json(&self,_cx:&mut Context,_d:usize)->Result<JsonValue,CodecError>{Ok(Nullable::Value(JsonNonNullValue::Bool(*self)))}
}
impl Scalar for crate::JsonNumber {
    fn from_json(v:JsonValue,cx:&mut Context)->Result<Self,CodecError>{match v {Nullable::Value(JsonNonNullValue::Number(v))=>Ok(v),_=>Err(cx.error("expected number"))}}
    fn to_json(&self,cx:&mut Context,_d:usize)->Result<JsonValue,CodecError>{cx.spend(self.as_str().len())?;Ok(Nullable::Value(JsonNonNullValue::Number(self.clone())))}
}
impl Scalar for crate::JsonInteger {
    fn from_json(v:JsonValue,cx:&mut Context)->Result<Self,CodecError>{let n=crate::JsonNumber::from_json(v,cx)?;cx.spend(n.as_str().len())?;n.as_str().parse().map_err(|_|cx.error("expected mathematical integer"))}
    fn to_json(&self,cx:&mut Context,_d:usize)->Result<JsonValue,CodecError>{cx.spend(self.as_str().len())?;Ok(Nullable::Value(JsonNonNullValue::Number(self.as_str().parse().map_err(|_|cx.error("invalid integer"))?)))}
}
macro_rules! native_integer {
    ($ty:ty,$convert:ident) => { impl Scalar for $ty {
        fn from_json(v:JsonValue,cx:&mut Context)->Result<Self,CodecError>{let n=crate::JsonInteger::from_json(v,cx)?;n.$convert().and_then(|v|<$ty>::try_from(v).ok()).ok_or_else(||cx.error("integer outside native range"))}
        fn to_json(&self,cx:&mut Context,_d:usize)->Result<JsonValue,CodecError>{let token=self.to_string();cx.spend(token.len())?;Ok(Nullable::Value(JsonNonNullValue::Number(token.parse().map_err(|_|cx.error("invalid native integer"))?)))}
    }};
}
native_integer!(u8,to_u128); native_integer!(u16,to_u128); native_integer!(u32,to_u128); native_integer!(u64,to_u128); native_integer!(u128,to_u128);
native_integer!(i8,to_i128); native_integer!(i16,to_i128); native_integer!(i32,to_i128); native_integer!(i64,to_i128); native_integer!(i128,to_i128);
impl Scalar for crate::Never {
    fn from_json(_v:JsonValue,cx:&mut Context)->Result<Self,CodecError>{Err(cx.error("false schema has no native value"))}
    fn to_json(&self,_cx:&mut Context,_d:usize)->Result<JsonValue,CodecError>{match *self {}}
}
impl Scalar for JsonValue {
    fn from_json(v:JsonValue,_cx:&mut Context)->Result<Self,CodecError>{Ok(v)}
    fn to_json(&self,cx:&mut Context,d:usize)->Result<JsonValue,CodecError>{copy_json(self,cx,d)}
}
impl Scalar for JsonNonNullValue {
    fn from_json(v:JsonValue,cx:&mut Context)->Result<Self,CodecError>{match v {Nullable::Value(v)=>Ok(v),Nullable::Null=>Err(cx.error("expected non-null value"))}}
    fn to_json(&self,cx:&mut Context,d:usize)->Result<JsonValue,CodecError>{
        // Borrowed traversal avoids cloning an unbounded subtree before checking limits.
        cx.step(d)?;
        let result=match self {
            Self::Bool(v)=>Self::Bool(*v),
            Self::String(v)=>{cx.spend(v.len())?;Self::String(v.clone())},
            Self::Number(v)=>{cx.spend(v.as_str().len())?;Self::Number(v.clone())},
            Self::Array(v)=>Self::Array(v.iter().enumerate().map(|(i,v)|cx.at_index(i,|cx|copy_json(v,cx,d+1))).collect::<Result<_,_>>()?),
            Self::Object(v)=>Self::Object(v.iter().map(|(k,v)|{cx.spend(k.len())?;Ok((k.clone(),cx.at(k,|cx|copy_json(v,cx,d+1))?))}).collect::<Result<_,CodecError>>()?),
        };Ok(Nullable::Value(result))
    }
}
