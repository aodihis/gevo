pub struct Redacted<T>(pub T);

impl<T> std::fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_string() {
        let r = Redacted("super-secret-key".to_string());
        assert_eq!(format!("{r:?}"), "<redacted>");
    }

    #[test]
    fn debug_hides_number() {
        let r = Redacted(12345u32);
        assert_eq!(format!("{r:?}"), "<redacted>");
    }

    #[test]
    fn inner_still_accessible() {
        let r = Redacted("hello".to_string());
        assert_eq!(r.0, "hello");
    }

    #[test]
    fn debug_inside_struct_also_hides() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Credentials {
            user: String,
            key: Redacted<String>,
        }
        let c = Credentials {
            user: "alice".into(),
            key: Redacted("pw123".into()),
        };
        let s = format!("{c:?}");
        assert!(s.contains("alice"));
        assert!(!s.contains("pw123"));
        assert!(s.contains("<redacted>"));
    }
}
