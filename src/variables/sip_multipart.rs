use super::EslArray;

/// A single part from a SIP multipart body
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartItem {
    pub mime_type: String,
    pub data: String,
}

/// Parses `variable_sip_multipart` ARRAY:: format.
///
/// Each ARRAY element is `"mime/type:body_data"`, split on the first `:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartBody(Vec<MultipartItem>);

impl MultipartBody {
    pub fn parse(s: &str) -> Option<Self> {
        let array = EslArray::parse(s)?;
        let items = array
            .items()
            .iter()
            .filter_map(|entry| {
                let (mime_type, data) = entry.split_once(':')?;
                Some(MultipartItem {
                    mime_type: mime_type.to_string(),
                    data: data.to_string(),
                })
            })
            .collect();
        Some(Self(items))
    }

    pub fn items(&self) -> &[MultipartItem] {
        &self.0
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multipart_body() {
        let input =
            "ARRAY::application/sdp:v=0\r\no=...|:application/pidf+xml:<presence>...</presence>";
        let body = MultipartBody::parse(input).unwrap();
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
        let body = MultipartBody::parse(input).unwrap();

        let pidf = body.by_mime_type("application/pidf+xml");
        assert_eq!(pidf, vec!["<pidf/>"]);

        let texts = body.by_mime_type("text/plain");
        assert_eq!(texts, vec!["hello", "world"]);

        let none = body.by_mime_type("application/json");
        assert!(none.is_empty());
    }

    #[test]
    fn non_array_returns_none() {
        assert!(MultipartBody::parse("not an array").is_none());
    }

    #[test]
    fn malformed_entries_skipped() {
        let input = "ARRAY::application/sdp:v=0|:no-colon-here|:text/plain:ok";
        let body = MultipartBody::parse(input).unwrap();
        assert_eq!(
            body.items()
                .len(),
            2
        );
        assert_eq!(body.items()[0].mime_type, "application/sdp");
        assert_eq!(body.items()[1].mime_type, "text/plain");
    }
}
