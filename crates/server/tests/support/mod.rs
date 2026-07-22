//! Shared server-test helper. Compiled once per test binary; not every
//! binary uses every item.
#![allow(dead_code)]

pub fn client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("authorization", "Bearer test-token".parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}
