# OAuth implicit and password flows are described, never executed

The OAuth lifecycle planner (`http_protocol/oauth.rs`) keeps declared implicit and resource-owner password flows in the descriptor, marked `deprecated_flow`, so docs and caller hooks can see them. Generated SDKs never run them, because RFC 9700 deprecates both grants. Authorization code with PKCE is the interactive default and client credentials is the service default. If a deprecated flow ever has to run, it must come from an explicitly selected compatibility helper, not the default runtime.
