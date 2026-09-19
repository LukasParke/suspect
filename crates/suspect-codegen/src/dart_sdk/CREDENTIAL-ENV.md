## Configured runtime credentials

This package explicitly binds environment **variable names** to source security
schemes. No credential values are read during generation or module import.

__BINDINGS__

With this policy, `Client(transport: IoTransport())` snapshots the mapped values
when the client is created. The source server remains the default. On the Dart
VM, a conditional I/O helper reads Dart's `Platform.environment` view. Portable
JavaScript/browser builds use an unavailable-environment stub. Missing, empty,
unavailable, oversized or invalid environment values remain missing credentials.
Anonymous operations remain usable; protected operations with no complete auth
alternative throw a bounded `ConfigurationException` before transport.

Every explicit `Credentials` object wins as a whole, including
`const Credentials()`, empty strings, null members and missing members. None is
supplemented from the environment. The parameter stays non-nullable:
`credentials: null` is a static type error; a dynamic null raises `TypeError`
before environment lookup. Existing native null/type errors are preserved.

For portable or mutable application configuration, provide
`environment: (String name) => values[name]`, where `values` is a
`Map<String, String>`. Each mapped variable is read once per client creation;
later map changes affect newly created clients, not existing ones. Reader errors
are treated as unavailable values and their messages are not exposed. Dart's
platform environment map itself is read-only and cached by the VM; injection is
the supported seam for mutable in-process configuration.

There is no per-request environment lookup, token acquisition or server rewrite.
Source OR/AND/anonymous alternatives and explicit `securityAlternative:` choices
continue through the normal credential machinery. API-key versus bearer behavior
comes from the bound source scheme, not its name.
