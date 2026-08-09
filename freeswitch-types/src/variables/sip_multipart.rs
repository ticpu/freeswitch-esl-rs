use super::EslArray;

/// A single part from a SIP multipart body
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct MultipartItem {
    /// MIME type (e.g. `application/sdp`).
    pub mime_type: String,
    /// Body content for this part.
    pub data: String,
}

impl MultipartItem {
    /// Create a new multipart item.
    pub fn new(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            data: data.into(),
        }
    }
}

/// Parses `variable_sip_multipart` ARRAY:: format.
///
/// Each ARRAY element is `"mime/type:body_data"`, split on the first `:`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MultipartBody(Vec<MultipartItem>);

impl MultipartBody {
    /// Parse a `variable_sip_multipart` value.
    ///
    /// Returns `Ok(None)` if the input is not `ARRAY::` formatted.
    /// Returns `Err` if an ARRAY entry is malformed (missing `:`
    /// separator between MIME type and body).
    pub fn parse(s: &str) -> Result<Option<Self>, String> {
        let array = match EslArray::parse(s) {
            Ok(a) => a,
            Err(super::EslArrayError::MissingPrefix) => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut items = Vec::with_capacity(
            array
                .items()
                .len(),
        );
        for entry in array.items() {
            let (mime_type, data) = entry
                .split_once(':')
                .ok_or_else(|| {
                    format!("malformed multipart ARRAY entry (missing ':'): {}", entry)
                })?;
            items.push(MultipartItem {
                mime_type: mime_type.to_string(),
                data: data.to_string(),
            });
        }
        Ok(Some(Self(items)))
    }

    /// All parsed parts.
    pub fn items(&self) -> &[MultipartItem] {
        &self.0
    }

    /// Number of parts.
    pub fn len(&self) -> usize {
        self.0
            .len()
    }

    /// Whether the body contains no parts.
    pub fn is_empty(&self) -> bool {
        self.0
            .is_empty()
    }

    /// Collect body data for all parts matching the given MIME type.
    pub fn by_mime_type(&self, mime: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|item| item.mime_type == mime)
            .map(|item| {
                item.data
                    .as_str()
            })
            .collect()
    }
}

impl IntoIterator for MultipartBody {
    type Item = MultipartItem;
    type IntoIter = std::vec::IntoIter<MultipartItem>;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .into_iter()
    }
}

impl<'a> IntoIterator for &'a MultipartBody {
    type Item = &'a MultipartItem;
    type IntoIter = std::slice::Iter<'a, MultipartItem>;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multipart_body() {
        let input =
            "ARRAY::application/sdp:v=0\r\no=...|:application/pidf+xml:<presence>...</presence>";
        let body = MultipartBody::parse(input)
            .unwrap()
            .unwrap();
        assert_eq!(
            body.items()
                .len(),
            2
        );
        assert_eq!(body.items()[0].mime_type, "application/sdp");
        assert_eq!(body.items()[0].data, "v=0\r\no=...");
        assert_eq!(body.items()[1].mime_type, "application/pidf+xml");
        assert_eq!(body.items()[1].data, "<presence>...</presence>");
    }

    #[test]
    fn by_mime_type_filtering() {
        let input = "ARRAY::text/plain:hello|:application/pidf+xml:<pidf/>|:text/plain:world";
        let body = MultipartBody::parse(input)
            .unwrap()
            .unwrap();

        let pidf = body.by_mime_type("application/pidf+xml");
        assert_eq!(pidf, vec!["<pidf/>"]);

        let texts = body.by_mime_type("text/plain");
        assert_eq!(texts, vec!["hello", "world"]);

        let none = body.by_mime_type("application/json");
        assert!(none.is_empty());
    }

    #[test]
    fn non_array_returns_none() {
        assert!(MultipartBody::parse("not an array")
            .unwrap()
            .is_none());
    }

    /// A part with no `:` is often a civic address or an SDP body, so the error
    /// names where it was and how long it was, never what it held.
    #[test]
    fn malformed_entry_error_names_position_not_content() {
        let input = "ARRAY::application/sdp:v=0|:123 Example St|:text/plain:ok";
        let msg = MultipartBody::parse(input)
            .unwrap_err()
            .to_string();
        assert!(
            !msg.contains("123 Example St"),
            "error quoted its input: {msg}"
        );
        assert!(
            msg.contains("entry 1"),
            "error does not name the entry: {msg}"
        );
    }
}
