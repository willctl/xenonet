//! What the doctor asks of Grok Build: the words for how many of its
//! sessions the map reads, left out while there are none.

use serde_json::Value;
use xenonet_text::text::{to_number, truthy};
use xenonet_text::units::plural;

pub fn counted(count: &Value) -> Option<String> {
    truthy(Some(count)).then(|| plural(to_number(Some(count)), "Grok Build session", None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn none_is_left_out() {
        assert_eq!((counted(&json!(2)).as_deref(), counted(&json!(0)), counted(&Value::Null)), (Some("2 Grok Build sessions"), None, None));
    }
}
