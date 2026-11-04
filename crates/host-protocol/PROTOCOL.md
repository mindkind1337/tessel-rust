# Tessel host protocol, version 1

The wire contract between a Tessel client (the Rust desktop, later the Vue app) and
`tessel-host` on a remote machine. `src/` implements it; `tests/fixtures/` holds one golden
frame of each shape; `tests/wire.rs` checks the rules below and `tests/fuzz.rs` fuzzes them.
The bodies of terminal, tree, git and editor messages come with their gates (3 to 6).

## Frames

A frame is a four-byte big-endian length, then that many bytes of UTF-8 JSON.

- At most 1 MiB, or less once negotiated (`frame_bytes`). A larger or zero length is refused
  from the header alone, before any payload is read or allocated.
- At most 16 nested objects and arrays, counted before parsing.
- No unknown field and no duplicate field, at any level of a frame's own structure.
- Numbers are integers within ±(2^53 − 1), bodies included: no fraction, no exponent, so a
  number means the same on both sides, in JavaScript too, and survives being sent again.
- Bytes (file content, terminal output) are canonical padded base64 of the standard alphabet,
  at most 64 KiB once decoded. One spelling per value: unpadded text or stray bits are refused.
- Paths below an opened root are arrays of names, each the base64 of its raw bytes: Linux names
  need not be UTF-8. No name is empty, `.`, `..`, or contains `/` or NUL; at most 255 bytes per
  name, 4096 for the whole path, 1024 names. `[]` is the root. Labels for people are separate.
- Identifiers chosen at random (`host_epoch`, stream ids, deduplication keys) are 32 lowercase
  hexadecimal digits.

A refusal never repeats what was sent: error details are fixed sentences of the host's, so they
can be logged without leaking a command, a file or a secret.

### From the client

A request, which gets exactly one reply:

```json
{"version":1,"request_id":9,"message":{"kind":"cancel","body":{"target":8}}}
```

An acknowledgment, which gets none: every event of that stream up to `seq` was received.

```json
{"version":1,"ack":{"stream":"89abcdef0123456789abcdef01234567","seq":42}}
```

`request_id` starts at 1 and goes up within a connection (gaps allowed), up to 2^53 − 1. A new
connection starts again from 1. `kind` is 1 to 64 characters of `a-z 0-9 . _`, starting with a
letter. `body` is left out when there is none.

### From the host

The reply to a request, either `ok` or `error`:

```json
{"version":1,"reply_to":1,"ok":{"kind":"hello_ok","body":{...}}}
{"version":1,"reply_to":7,"error":{"code":"capability_not_negotiated","retryable":false}}
```

An event of a stream (sequence numbers start at 1 and go up by exactly 1):

```json
{"version":1,"stream":{"id":"89abcdef0123456789abcdef01234567","seq":42},"event":{"kind":"terminal.output","body":{"data":"w6kbWzBtCg=="}}}
```

An error about the whole connection, after which the host closes it:

```json
{"version":1,"error":{"code":"bad_frame","retryable":false,"detail":"request identifier must increase"}}
```

## Negotiation

The first request must be `hello`. Before it, anything else, an acknowledgment included,
closes the connection.

```json
{"version":1,"request_id":1,"message":{"kind":"hello","body":{
  "versions":[1],
  "capabilities":["tree","git","future.optional"],
  "budgets":{"frame_bytes":1048576,"terminal_ring_bytes":2097152,"total_ring_bytes":16777216,
             "stream_window_bytes":2097152,"in_flight":32}}}}
```

- `versions`: those the client speaks. The host answers with the highest it shares, or refuses
  with `unsupported_version` and closes.
- `capabilities`: `pty`, `tree`, `git`, `editor`, `tunnel.reverse`, each negotiated on its own.
  Names the host does not know are ignored, so a newer client can ask for more. The answer lists
  those granted, once each. A client may take tree and git and keep its terminals elsewhere.
- `budgets`: each side's limits; the smaller of each pair applies. `frame_bytes` 1 KiB to 1 MiB;
  `terminal_ring_bytes` ≤ `total_ring_bytes` ≤ 64 MiB; `stream_window_bytes` from `frame_bytes`
  to 16 MiB; `in_flight` 1 to 256. The slow-host default is the one above.

`hello_ok` carries `version`, `capabilities`, `budgets` and `host_epoch`, the identity of this
run of the daemon: another epoch after a reconnect means what the client knew is gone.

## Requests

- Core kinds, always allowed after hello: `hello` (refused: already accepted), `ping` → `pong`,
  `cancel` → `done`.
- Every other kind belongs to a capability by its first word: `terminal.*` → pty, `tree.*` and
  `root.*` → tree, `file.*` → editor, `git.*` → git, `tunnel.*` → tunnel.reverse. A kind of a
  capability not negotiated is refused with `capability_not_negotiated`; a kind no capability
  knows, or one the host does not implement, with `unsupported_kind`. Neither closes anything.
- At most `in_flight` requests wait for their reply at once; one more is refused with `busy`,
  which is retryable.
- `cancel` names an earlier request still waiting: that request then gets `cancelled` as its one
  reply, and the cancel itself gets `done`. A target already answered or unknown: `not_found`.
- An operation with an effect (creating a terminal) carries a deduplication key chosen by the
  client. Retrying with the same key, even over a new connection, returns the first outcome
  instead of doing it again, for 10 minutes after it finished. While the table of keys is full of
  live entries, a new key is refused with `busy` rather than forgetting one.

## Streams

Each event has a stream id and a sequence number. The client applies an event only if it is the
next one: an older one is a duplicate (a replay overlapping what it saw) and is dropped; a jump
is a gap, never applied, and needs a snapshot or a reset. The host keeps at most
`stream_window_bytes` unacknowledged per stream (one event is always allowed when nothing is in
flight); a slow client only slows its own stream. An acknowledgment for an event never sent
closes the connection; an older one than already seen is ignored.

## Errors

| code | meaning | retryable |
| --- | --- | --- |
| `bad_frame` | the frame itself is broken; the connection closes | no |
| `bad_request` | a well-formed frame asking something invalid | no |
| `unsupported_version` | no shared version; the connection closes | no |
| `unsupported_kind` | a kind this host does not handle | no |
| `not_negotiated` | something other than hello came first; the connection closes | no |
| `capability_not_negotiated` | that capability was not granted | no |
| `busy` | too many requests or entries at once | yes |
| `cancelled` | the request was cancelled | no |
| `not_found`, `permission_denied`, `conflict`, `too_large` | as named; `conflict`: someone else changed it first | no |
| `gap` | the history needed to resume is gone: take a snapshot or start over | no |
| `epoch_mismatch` | the daemon restarted since | no |
| `timeout` | took too long | yes |
| `internal` | the host failed | no |

A client reads a code it does not know as `unknown`.

## Compatibility matrix

| client sends | when | host does |
| --- | --- | --- |
| hello, versions `[1]` | first | `hello_ok`, version 1 |
| hello, versions `[2, 1]` | first | `hello_ok`, version 1 |
| hello, versions `[2]` | first | reply `unsupported_version`, closes |
| a frame with `"version":2` | any time | connection error `unsupported_version`, closes |
| hello with a malformed body | first | reply `bad_request`, closes |
| ping, any request, or an ack | before hello | reply (or connection error) `not_negotiated`, closes |
| hello asking for `future.optional` | first | ignored, not in `hello_ok` |
| hello again | after hello | reply `bad_request` |
| `future.thing` | after hello | reply `unsupported_kind` |
| `git.status` without git granted | after hello | reply `capability_not_negotiated` |
| `tree.subscribe` with tree granted | after hello | handled; one reply later |
| one request more than `in_flight` | after hello | reply `busy` (retryable) |
| cancel of a waiting request | after hello | `done`; the target gets `cancelled` |
| cancel of an answered or unknown request | after hello | reply `not_found` |
| a `request_id` not above the last one | any time | connection error `bad_frame`, closes |
| a fraction or a number beyond ±(2^53 − 1) | any time | connection error `bad_frame`, closes |
