# An offline run answers each request with the recording of that request

`suspect test run --cassette` matches every request to a recorded exchange by method, path and query; scheme and host are ignored, so a cassette recorded through a gateway replays against any base URL. Among matching recordings, one not yet served with the same request body wins, then the earliest not yet served, so a request repeated within a run (read, delete, read again) walks its recordings in order; once they are used up the latest answer stands. A request with no recording is a transport failure that names it.

## Considered Options

- Serve cassette entries in file order, ignoring the request, as the transport first did: rejected. Workflows run concurrently, so requests do not arrive in the order they were recorded; the documented record-then-replay flow failed on the bundled example, with each workflow reading another's response.
- Reuse the gateway's `ReplayIndex` (latest recording of a request wins, stateless): rejected for the test transport. A suite that reads a resource before and after changing it needs both recordings, in order.

## Consequences

The gateway's `replay` mode and the test transport share the request key but not the state: the gateway serves any number of clients for as long as it runs, so it stays stateless.
