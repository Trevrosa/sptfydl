use serde_json::Value;

pub trait ValueExt {
    fn try_to_string(&self) -> Option<String>;
}

impl ValueExt for Value {
    fn try_to_string(&self) -> Option<String> {
        Some(self.as_str()?.to_string())
    }
}
