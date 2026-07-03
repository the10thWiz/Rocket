use crate::http::RawStr;

#[derive(Debug, Clone)]
pub struct QuerySegment {
    /// The name of the parameter or just the static string.
    pub value: String,
    /// This is a `<a>`.
    pub dynamic: bool,
}

impl QuerySegment {
    pub fn from(segment: &RawStr) -> Self {
        let mut value = segment;
        let mut dynamic = false;

        if segment.starts_with('<') && segment.ends_with('>') {
            dynamic = true;
            value = &segment[1..(segment.len() - 1)];
        }

        QuerySegment { value: value.to_string(), dynamic }
    }
}

#[derive(Debug, Clone)]
pub struct Segment {
    /// The name of the parameter or just the static string.
    pub value: String,
    // /// This is a `<a>`.
    // pub dynamic: bool,
    /// This is a `<a..>`.
    pub dynamic_trail: bool,
    pub dynamic: Option<(String, String)>,
}

fn split_name(segment: &RawStr) -> Option<(&RawStr, &RawStr, &RawStr)> {
    let start = segment.find('<')?;
    let end = segment[start..].find('>')? + start;
    Some((&segment[..start], &segment[start+1..end], &segment[end+1..]))
}

impl Segment {
    pub fn from(segment: &RawStr) -> Self {
        let mut value = segment;
        let mut dynamic = None;
        let mut dynamic_trail = false;

        if let Some((prefix, name, postfix)) = split_name(segment) {
            value = name;
            dynamic = Some((prefix.to_string(), postfix.to_string()));
            
            if name.ends_with("..") {
                dynamic_trail = true;
            }
        }
        // segment.starts_with('<') && segment.ends_with('>') {
        //     dynamic = true;
        //     value = &segment[1..(segment.len() - 1)];

        //     if value.ends_with("..") {
        //         dynamic_trail = true;
        //         value = &value[..(value.len() - 2)];
        //     }
        // }

        Segment { value: value.to_string(), dynamic, dynamic_trail }
    }
}
