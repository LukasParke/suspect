# Generated clients make exactly one HTTP exchange per call

A generated operation sends one request and returns whatever comes back. The SDK's default transport in every backend follows no redirects (they are errors), ignores ambient proxy settings, leaves automatic decompression off, sends no cookies or referer, adds no retries, and gives request bodies no replay factory. Bundled adapters (reqwest, URLSession, fetch, httpx, net/http) are hardened to keep this even with caller-supplied builders. Codecs must see the exact response bytes. Credentials must not reach another host through a redirect or a proxy the caller did not choose. OpenAPI does not declare retry safety, so a default retry on POST/PATCH could duplicate side effects. Retry, timeout, proxy and redirect policy belongs to the caller through the injectable transport.

## Consequences

The one exception is the opt-in `sdk_defaults` OAuth lifecycle (ADR-0011): one coordinated token refresh and one replay after a 401 on a request it authenticated, never for streams. Users coming from SDKs that retry 5xx by default must add their own retry policy.
