//! A cheap check run before a script reaches Boa: Boa's parser recurses once per nested
//! bracket, so a script nested thousands deep would overflow the stack and take the browser
//! down. Such scripts are refused like syntax errors instead. Shared with the fuzzers.

/// The bracket depth of a script if it's over `limit`. Strings, template literals and
/// comments are skipped; regular expressions aren't, which can only overcount slightly.
pub fn too_deep(code: &str, limit: usize) -> Option<usize> {
    let b = code.as_bytes();
    if b.len() < limit {
        return None;
    }
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => {
                depth += 1;
                max = max.max(depth);
                if max > limit {
                    return Some(max);
                }
            }
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    } else if q != b'`' && b[i] == b'\n' {
                        break;
                    }
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_scripts_are_caught_and_normal_ones_pass() {
        let deep = format!("f({}{})", "(".repeat(3000), ")".repeat(3000));
        assert!(too_deep(&deep, 1000).is_some());
        let arr = format!("var a = {}{};", "[".repeat(1500), "]".repeat(1500));
        assert!(too_deep(&arr, 1000).is_some());
        // Brackets inside strings and comments don't count.
        let s = format!("var s = \"{}\"; /* {} */ // {}\nf();", "(".repeat(5000), "[".repeat(5000), "{".repeat(5000));
        assert!(too_deep(&s, 1000).is_none());
        let normal = "function f(a) { return [a, {b: (1 + 2)}]; }\n".repeat(500);
        assert!(too_deep(&normal, 1000).is_none());
    }
}
