use std::fmt;

const ARRAY_HEADER: &str = "ARRAY::";
const ARRAY_SEPARATOR: &str = "|:";

/// Parses FreeSWITCH `ARRAY::item1|:item2|:item3` format
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EslArray(Vec<String>);

impl EslArray {
    /// Parse an `ARRAY::` formatted string. Returns `None` if the prefix is missing.
    pub fn parse(s: &str) -> Option<Self> {
        let body = s.strip_prefix(ARRAY_HEADER)?;
        let items = body
            .split(ARRAY_SEPARATOR)
            .map(String::from)
            .collect();
        Some(Self(items))
    }

    /// Create a new array from a vec of items.
    pub fn new(items: Vec<String>) -> Self {
        Self(items)
    }

    /// Append an item to the end.
    pub fn push(&mut self, value: String) {
        self.0
            .push(value);
    }

    /// Prepend an item to the front.
    pub fn unshift(&mut self, value: String) {
        self.0
            .insert(0, value);
    }

    /// The parsed array items.
    pub fn items(&self) -> &[String] {
        &self.0
    }

    /// Number of items in the array.
    pub fn len(&self) -> usize {
        self.0
            .len()
    }

    /// Returns `true` if the array has no items.
    pub fn is_empty(&self) -> bool {
        self.0
            .is_empty()
    }
}

impl fmt::Display for EslArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ARRAY_HEADER)?;
        for (i, item) in self
            .0
            .iter()
            .enumerate()
        {
            if i > 0 {
                f.write_str(ARRAY_SEPARATOR)?;
            }
            f.write_str(item)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_item() {
        let arr = EslArray::parse("ARRAY::hello").unwrap();
        assert_eq!(arr.items(), &["hello"]);
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn parse_multiple_items() {
        let arr = EslArray::parse("ARRAY::one|:two|:three").unwrap();
        assert_eq!(arr.items(), &["one", "two", "three"]);
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn parse_non_array_returns_none() {
        assert!(EslArray::parse("not an array").is_none());
        assert!(EslArray::parse("").is_none());
        assert!(EslArray::parse("ARRAY:").is_none());
    }

    #[test]
    fn display_round_trip() {
        let input = "ARRAY::one|:two|:three";
        let arr = EslArray::parse(input).unwrap();
        assert_eq!(arr.to_string(), input);
    }

    #[test]
    fn display_single_item() {
        let arr = EslArray::parse("ARRAY::only").unwrap();
        assert_eq!(arr.to_string(), "ARRAY::only");
    }

    #[test]
    fn empty_items_in_array() {
        let arr = EslArray::parse("ARRAY::|:|:stuff").unwrap();
        assert_eq!(arr.items(), &["", "", "stuff"]);
    }

    #[test]
    fn test_new() {
        let arr = EslArray::new(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(arr.items(), &["a", "b", "c"]);
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_push() {
        let mut arr = EslArray::new(vec!["first".into()]);
        arr.push("second".into());
        arr.push("third".into());
        assert_eq!(arr.items(), &["first", "second", "third"]);
        assert_eq!(arr.to_string(), "ARRAY::first|:second|:third");
    }

    #[test]
    fn test_unshift() {
        let mut arr = EslArray::new(vec!["last".into()]);
        arr.unshift("middle".into());
        arr.unshift("first".into());
        assert_eq!(arr.items(), &["first", "middle", "last"]);
        assert_eq!(arr.to_string(), "ARRAY::first|:middle|:last");
    }
}
