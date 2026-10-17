/// Join URL segments, ignoring leading and trailing slashes on each and dropping empty ones.
pub fn join_url(segments: &[&str]) -> String {
    let mut url = String::new();
    for seg in segments {
        let seg = seg.trim_matches('/');
        if !seg.is_empty() {
            if !url.is_empty() {
                url.push('/');
            }
            url.push_str(seg);
        }
    }
    url
}

/// Ensure a trailing slash: the genai client needs the base URL in directory form.
pub fn ensure_trailing_slash(mut url: String) -> String {
    if !url.is_empty() && !url.ends_with('/') {
        url.push('/');
    }
    url
}
