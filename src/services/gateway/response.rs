use std::convert::Infallible;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Response;

use super::ProxyBody;

pub(super) fn text_response(status: u16, msg: &'static str) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from(msg))
        .map_err(|_: Infallible| unreachable!())
        .boxed();
    Response::builder().status(status).body(body).expect("valid response")
}

pub(super) fn json_response(status: u16, body: Vec<u8>) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from(body))
        .map_err(|_: Infallible| unreachable!())
        .boxed();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body)
        .expect("valid response")
}
