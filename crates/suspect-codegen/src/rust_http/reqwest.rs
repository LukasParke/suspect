//! Generated reqwest transport adapter emitted as `crate::reqwest_transport`
//! behind the `reqwest-rustls` feature.
//!
//! The adapter wraps a hardened `reqwest::Client`: redirects disabled,
//! retries disabled (including reqwest's default protocol-NACK retry),
//! automatic decompression disabled even under feature unification, no
//! cookies, and no referer. Policies are applied on top of any
//! caller-provided builder so an already-built client whose redirect policy
//! cannot be enforced is never accepted. No implicit credentials, cookies,
//! referer, proxy or default timeout are configured; callers cancel by
//! dropping futures inside their own executor.
//!
//! Raw response header values are preserved; `reqwest` normalizes header
//! names to lowercase. The adapter rejects any URL normalization and any
//! final URL that differs from the prepared request. Model-only builds never
//! load this module or its dependencies. No unsafe code.

#![forbid(unsafe_code)]

use crate::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use reqwest::{Client, ClientBuilder, Method, Url};
use std::io;

/// Recommended asynchronous transport over a pinned, hardened reqwest
/// client. The wrapped connection state is owned; no work is detached.
pub struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
    /// Build the hardened default client: no redirects, no retries, no
    /// automatic decompression, no cookies, no referer, no timeout.
    ///
    /// # Errors
    /// Returns the underlying `reqwest::Error` when the TLS backend or
    /// client configuration cannot be initialized.
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::from_builder(reqwest::Client::builder())
    }

    /// Apply the mandatory hardening policies to a caller-provided builder
    /// and build the client. Caller pool, TLS and timeout settings are
    /// preserved; proxy routing, redirect, retry, referer and decompression policies are
    /// overridden unconditionally, so a pre-built client whose policies are
    /// already fixed cannot be smuggled in.
    ///
    /// # Errors
    /// Returns the underlying `reqwest::Error` when the client cannot be
    /// built.
    pub fn from_builder(builder: ClientBuilder) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: builder
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .referer(false)
                .no_brotli()
                .no_deflate()
                .no_gzip()
                .no_zstd()
                .build()?,
        })
    }
}

impl Transport for ReqwestTransport {
    type Body = ReqwestResponseBody;

    async fn send(
        &self,
        request: Request,
    ) -> Result<TransportResponse<ReqwestResponseBody>, BoxError> {
        let url = Url::parse(&request.url).map_err(|error| Box::new(error) as BoxError)?;
        if url.as_str() != request.url {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reqwest adapter rejected a normalized request URL",
            )
            .into());
        }
        let method = Method::from_bytes(request.method.as_bytes())
            .map_err(|error| Box::new(error) as BoxError)?;
        let mut wire = reqwest::Request::new(method, url);
        {
            let headers = wire.headers_mut();
            for (name, value) in &request.headers {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|error| Box::new(error) as BoxError)?;
                let value = reqwest::header::HeaderValue::from_bytes(value)
                    .map_err(|error| Box::new(error) as BoxError)?;
                headers.append(name, value);
            }
        }
        if let Some(body) = request.body {
            *wire.body_mut() = Some(reqwest::Body::from(body));
        }
        let response = self
            .client
            .execute(wire)
            .await
            .map_err(|error| Box::new(error) as BoxError)?;
        if response.url().as_str() != request.url {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reqwest final URL does not match the prepared request URL",
            )
            .into());
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec()))
            .collect();
        Ok(TransportResponse {
            status,
            headers,
            body: ReqwestResponseBody { response },
        })
    }
}

/// Streaming body over one reqwest response; dropping it cancels the
/// transfer. Chunks are delivered as received, without buffering the whole
/// body and without decompression.
pub struct ReqwestResponseBody {
    response: reqwest::Response,
}

impl ResponseBody for ReqwestResponseBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        match self.response.chunk().await {
            Ok(Some(chunk)) => Ok(Some(chunk.to_vec())),
            Ok(None) => Ok(None),
            Err(error) => Err(Box::new(error)),
        }
    }
}
