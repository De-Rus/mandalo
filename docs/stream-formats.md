# Streams: `.ws`, `.mqtt`, and SSE in `.http`

A stream is a saved request like any other. It lives in a collection, it is
addressed as `path#index`, it renames, moves, exports and imports the same way,
and it carries `{{vars}}` and a `< {% … %}` script. What it does not have is a
single body and a single response — it has a **connection** and a list of
**named messages** you can send over it.

Three protocols, two new file types:

| Protocol | File | Why |
| --- | --- | --- |
| WebSocket | `.ws` | Its own handshake, its own subprotocol negotiation. |
| MQTT | `.mqtt` | No URL path, no headers; topics, qos and a client id instead. |
| Server-sent events | `.http` | **It is an HTTP GET.** It needs no file type of its own. |

The shape of `.ws` and `.mqtt` is the [`.grpc` format](grpc-format.md) with a
different request line and a message list, which is itself the
[`.http` format](http-format.md) with a different request line. `###`
separators, `@vars`, `#` comments and request identity are the same everywhere
and are documented in [http-format.md](http-format.md).

## `.ws`

```ws
### Chat socket
wss://{{host}}/socket
subprotocol: chat.v2
authorization: Bearer {{token}}

>> subscribe
{"op": "sub", "channel": "prices"}

>> ping
{"op": "ping"}
```

The request line is the url and nothing else — no method, because a websocket
has one. Every header-style line under it is an **HTTP header sent with the
handshake**, exactly as written, except the three reserved keys below. There is
no typed auth block: `Authorization: Bearer {{token}}` is the header that goes
on the wire, and writing it is writing it.

| Reserved key | Also spelled | Means |
| --- | --- | --- |
| `subprotocol:` | `subprotocols:` | One offered subprotocol; repeatable, in preference order. |
| `auto-reconnect:` | `autoReconnect:` | `true`/`false` (`on`/`off`, `yes`/`no`). Off by default. |
| `ping-interval:` | `pingInterval:` | Whole seconds between client pings. `0`, the default, sends none. |

## `.mqtt`

```mqtt
### Sensors
mqtt://{{broker}}:1883
client-id: mandalo-{{trace}}
username: {{user}}
keep-alive: 30
subscribe: sensors/#; qos=1

>> report
topic: sensors/{{room}}/temp
qos: 1

{"c": 21.5}
```

MQTT has no headers at all, so `.mqtt` accepts a **closed set** of connection
options and refuses everything else rather than sending it somewhere. The url
may be `mqtt://`, `mqtts://`, `ws://` or `wss://` — the last two are MQTT over
websockets, which is what a broker behind a single TLS port usually offers.

| Option | Also spelled | Means |
| --- | --- | --- |
| `client-id:` | `clientId:` | The MQTT client id. A random `mandalo-…` one is used when it is absent. |
| `username:` | — | The CONNECT username. MQTT signs in in its CONNECT packet, not with a header. |
| `password:` | — | **`{{variable}}` only** — see below. |
| `keep-alive:` | `keepAlive:` | Whole seconds; `60` by default. |
| `clean-session:` | `cleanSession:` | `true` by default. |
| `protocol-version:` | `protocolVersion:` | `3.1.1` (the default) or `5`. |
| `subscribe:` | `subscriptions:` | One topic filter, repeatable. `; qos=1` is the same `; key=value` parameter a form-data file line takes. |

## Messages

A socket's body is a list, not a value, so each message opens with `>> name`:

```
>> report
topic: sensors/{{room}}/temp
qos: 1

{"c": 21.5}
```

The name is how you send it — `mandalo listen … --message report`, or the send
button next to it in the app — so two messages in one block may not share one.
Everything after the option lines is the payload, verbatim, until the next `>>`
or the next `###`.

`.mqtt` messages take `topic:`, `qos:` and `retain:`, and `topic:` is required —
a publish has nowhere to go without one. A `.ws` message takes no options at
all: it is a payload and nothing else, and writing `topic:` under a `>>` in a
`.ws` file is refused rather than sent as the first line of the frame.

The option lines end at the first line that is not one of those keys, so a
payload that happens to start `status: ok` is still a payload. A payload whose
first line reads exactly `topic:`, `qos:` or `retain:` needs a blank line above
it.

## Server-sent events

SSE is an HTTP GET that asks for `text/event-stream`, so it is written as one:

```http
### Prices
GET {{baseUrl}}/prices/stream
Accept: text/event-stream
Last-Event-ID: 42
```

**The `Accept` header is the marker.** Not a comment, not an invented key — the
header a browser sends and a server keys off. A request that carries it is a
stream; one that does not is a plain request. Nothing else changes: same file,
same folder, same `###` blocks as every other HTTP request.

`Last-Event-ID` is likewise the real header, so it is written as one — and read
back as the `lastEventId` field, because the transport carries the last id
forward across reconnects and would otherwise send it twice. Same for `Accept`:
the transport sends it, so the saved one marks the request without travelling
twice.

One directive, because there is no header for it:

```http
# @reconnect off
```

`on` is the default. It only means something on a request that accepts
`text/event-stream`; writing it anywhere else is refused, and a stream may not
be a `POST`.

## No password in the file

`password:` takes a `{{variable}}` and refuses a literal, on the way in and on
the way out:

> a password may not be written into a request file — put it in an environment
> as `password = { secret = true }` and write `password: {{password}}` here, so
> the value stays on this machine

A collection is committed and shared. The workspace already has the right place
for a credential — an environment entry marked `secret = true`, whose value is
read from `MANDALO_SECRET__<ENV>__<KEY>` or from this machine's
`secrets.toml` and never enters the file — so a request file has no reason to
hold one, and a hand-edited file cannot smuggle one past the parser.

`username:` is not held to the same rule, and neither is an `Authorization:`
header in a `.ws` file. That is deliberate: they are the same lines a `.http`
file has always allowed, holding them to a stricter standard here would make the
two formats disagree about the same header, and a username is not on its own a
credential. Everything in the workspace is still swept by `mandalo scan`, which
is what catches the general case.

## `{{vars}}` and scripts

`{{vars}}` work everywhere: the url, the headers, the connection options, the
message payloads and the message topics. They resolve from the file's own
`@vars` first and the environment second, exactly as in `.http`.

A **`< {% … %}` pre-connect script** runs once before the socket opens, on the
same `pm.*` engine as everywhere else
([postman-compatibility.md](postman-compatibility.md)). It sees the connection
as `pm.request` — url, headers, and the method the tree shows (`WS`, `MQTT` or
`GET`) — and what it writes with `pm.environment.set` is what the connection
interpolates against. The writes last for that connection; a stream is not a
suite step, so nothing is persisted after it.

A **`> {% … %}` response script is refused**, loudly, in all three formats:

> a websocket has no single response, so a `> {% … %}` script has nothing to run
> against — read the events with `mandalo listen --json` instead

There is no honest thing to hand such a script. A socket produces a sequence
with no end: `pm.response` would have to mean "the first message", or "the last
one", or "whichever arrived when the script ran", and each of those is a
different silent answer. Assert on the event stream instead, which
`mandalo listen --json` prints one JSON event per line.

## The model

One flat `stream` object, whichever protocol it is. Which fields apply is
decided by `kind`, so the protocol never appears twice and the two can never
disagree. Every field is optional and absent means "the transport's default".

```ts
type RequestKind = "http" | "graphql" | "grpc" | "websocket" | "sse" | "mqtt";

interface SavedRequest {
  // …everything it already had…
  kind: RequestKind;
  stream?: SavedStream | null;   // null for every non-stream kind
}

interface SavedStream {
  subprotocols?: string[];                    // ws
  autoReconnect?: boolean;                    // ws (default false), sse (default true)
  pingIntervalMs?: number;                    // ws
  lastEventId?: string | null;                // sse
  clientId?: string | null;                   // mqtt
  username?: string | null;                   // mqtt
  password?: string | null;                   // mqtt — "{{var}}" only
  cleanSession?: boolean;                     // mqtt (default true)
  keepAliveSecs?: number;                     // mqtt (default 60)
  subscriptions?: { topic: string; qos?: number }[];  // mqtt
  protocolVersion?: "3.1.1" | "5";            // mqtt
  messages?: SavedMessage[];                  // ws, mqtt
}

interface SavedMessage {
  id: string;        // derived from the request id and the position; stable, not stored
  name: string;      // unique inside one request
  message: Outgoing; // exactly what `stream_send` takes
}
```

`message` is an `Outgoing` on purpose: the saved message and the live send are
the same value, so a chip in the sidebar sends what the file holds without a
translation step in between. A `.ws` file writes `{kind:"text"}` and a `.mqtt`
file writes `{kind:"publish"}`; any other variant is refused on save rather than
dropped.

`id` is derived — `<requestId>-<position>` — so the file carries no identifier of
its own and two saves of the same file produce the same ids.

## Running one

```sh
mandalo listen wss://echo.example/socket --send hola      # a raw url, unchanged
mandalo listen mock streams/chat.ws#0 --message hola      # a saved stream
mandalo listen mock 'streams/chat.ws#Chat socket' --env local
```

Two positional arguments mean "collection, then request path"; one means a url.
`--message` names a message the file already holds; `--send` is still literal
text; `--header`, `--topic` and `--reconnect` add to what the file says rather
than replacing it.

`mandalo run` skips streams. A suite is a sequence of send-and-assert steps and
a connection that stays open is not one — it would never reach the next request.
They are listed by `mandalo ls` and run one at a time with `mandalo listen`.

## What the formats cannot express

| Not expressible | What happens |
| --- | --- |
| A `> {% … %}` response script | Refused, with the message above. |
| An HTTP header on an MQTT connection | ``an mqtt connection carries no headers — it signs in with client-id, username and password`` |
| A topic, qos or retain on a websocket message | ``"topic" reads like a Mándalo key but means nothing here — this line accepts no options at all`` |
| A key that reads structural — `url:`, `message:`, `send:`, `subscribe:` in a `.ws` file | Refused rather than sent as an HTTP header. |
| Declarative `[[tests]]` and `[[captures]]` | ``a .ws file cannot carry declarative tests or captures`` |
| A per-request description **as an editable field** | Kept in the `#` comments above the request, like `.http` and `.grpc`. |
| A binary message, a saved `subscribe`/`unsubscribe`/`ping` | ``a .ws file writes text messages, and this message is not one — send it from the connection instead``. Subscriptions belong on the connection (`subscribe:`); the rest are live actions. |
| A literal password | Refused; see above. |
| Per-request transport limits (buffer sizes, timeouts, backoff) | Not written, and nothing is lost: they are transport tuning, not part of the request, and they are set on the live connection when it opens. |
| An MQTT last-will, or MQTT 5 properties | Not modelled. MQTT 5 itself is refused at connect time until it is wired up. |
| An `autoReconnect` for MQTT | There is none: the MQTT transport always reconnects. The option is a websocket and SSE one. |
| Per-request stream limits (buffer sizes, timeouts, backoff) | Not written; every connection uses the same defaults. Nothing is lost on save because nothing sets them per request. |
| A subscription made *after* connecting | The file lists what to subscribe to at connect. Sending an unsubscribe or a late subscribe is a live action, not a saved one. |

An option the file never wrote stays unwritten: absent means "whatever the
transport does by default" — `auto-reconnect` off for a websocket, on for SSE —
so a file that says nothing means the same thing on both sides.

Saving a request that needs one of the first six fails with the message shown.
Nothing is dropped in silence.

## Importing from Postman

Postman's WebSocket and MQTT requests are imported: the url arrives, and for a
websocket the headers arrive with it. The messages do not, because a Postman
export does not contain them — they live in its console, not in the collection
file. The import says so per request rather than writing a file that looks
finished:

> `Chat: imported as a websocket with its url and headers — a Postman export
> carries no saved messages, so add the `>> name` blocks yourself`

> `Sensors: imported as an mqtt connection with its url only — a Postman export
> carries no topics, client id or credentials, so add the `subscribe:`,
> `client-id:` and `>> name` lines yourself`

A Socket.IO request arrives as the plain websocket it is built on; Mándalo does
not speak Socket.IO's framing on top of it.
