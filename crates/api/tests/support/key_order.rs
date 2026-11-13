//! Reads JSON key order from a body, which `serde_json::Value` loses.

/// The keys, in body order, of the first JSON object that starts right after `marker` (such as
/// `"@attr":`), joined with commas.
pub(crate) fn key_order(body: &str, marker: &str) -> String {
    let start = body.find(marker).unwrap() + marker.len();
    let (mut depth, mut in_string, mut escaped) = (0_usize, false, false);
    let (mut keys, mut current) = (Vec::new(), String::new());
    for c in body[start..].chars() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ if depth == 1 => current.push(c),
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            ':' if depth == 1 => keys.push(std::mem::take(&mut current)),
            ',' if depth == 1 => current.clear(),
            '{' | '[' => depth += 1,
            '}' | ']' if depth == 1 => break,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    keys.join(",")
}
