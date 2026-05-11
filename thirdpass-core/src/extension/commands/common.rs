use crate::extension::process::ProcessResult;
use anyhow::Result;

pub fn communicate_result<T: serde::Serialize>(result: Result<T>) -> Result<()> {
    let result = match result {
        Ok(r) => ProcessResult {
            ok: Some(r),
            err: None,
        },
        Err(e) => ProcessResult {
            ok: None,
            err: Some(e.to_string()),
        },
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
