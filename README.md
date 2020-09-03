# Spectacles Proxy

The Spectacles proxy is responsible for handling all of the data interactions with Discord.

- [x] Ratelimited REST calls to Discord
- [ ] Caching REST responses
- [ ] Caching ingested data from the [gateway](https://github.com/spec-tacles/gateway)

## Usage

The proxy communicates with other services using the [Spectacles spec](https://github.com/spec-tacles/spec) over AMQP.

- JS: [`brokers.js`](https://github.com/spec-tacles/brokers.js)
- C#: [`Spectacles.NET`](https://github.com/spec-tacles/Spectacles.NET)
- Go: [`spec-tacles/go`](https://github.com/spec-tacles/go)
- Rust: [`rustacles`](https://github.com/spec-tacles/rustacles)

The proxy can be configured with the following options. This file must be called `proxy.toml` and exist in the CWD. Alternatively, specify the values in the environment variables named adjacently. The following example contains the default values.

```toml
timeout = "" # TIMEOUT

[redis]
url = "redis://localhost:6379" # REDIS_URL

[amqp]
url = "amqp://localhost:5672/%2f" # AMQP_URL
group = "rest" # AMQP_GROUP
# subgroup = "foo" # AMQP_SUBGROUP
event = "REQUEST" # AMQP_EVENT
cancellation_event = "CANCEL" # AMQP_CANCELLATION_EVENT
```

### Timeout

The timeout is a human-readable duration (e.g. 2min). It applies for the entire duration of the request, including time paused for ratelimiting. Once the timeout occurs, the proxy will attempt to stop the request; however, it's possible for the data to be sent to Discord and the timeout to occur during the response, meaning that your client will receive the error but the request will have succeeded. This is done to protect against indefinitely hung requests in case Discord doesn't respond.

### Cancellation

Requests can be explicitly cancelled by the client by publishing an event to the `amqp.cancellation_event` queue with the correlation ID of the request to cancel. This ID must be valid UTF-8.

### Request Format

Requests can be made by publishing on the specified event to the specified group. The data must be serialized in MessagePack format.

```json
{
	"method": "GET",
	"path": "/users/1234",
	"query": {
		"foo": "bar"
	},
	"body": [],
	"headers": {
		"def": "uvw"
	}
}
```

`query`, `body`, and `headers` are optional. Body must be binary data.

### Response Format

The response is returned on the callback queue in the following MessagePack format.

```json
{
	"status": 0,
	"body": ...
}
```

#### Response Status

**Status**|**Description**
-----:|-----
0|Success
1|Unknown error
2|Invalid request format (non-JSON)
3|Invalid URL path
4|Invalid URL query
5|Invalid HTTP method
6|Invalid HTTP headers
7|Request failure
8|Request timeout

#### Response Body

For a successful call (status 0), the body will be an object representing the HTTP response:

```json
{
	"status": 200,
	"headers": {
		"foo": "bar"
	},
	"url": "https://discord.com/api/v6/users/4567",
	"body": []
}
```

`url` represents the full, final URL of the request. `body` is the binary response body from the server.

For an unsuccessful status code (non-zero status), the body will be a string describing the error.
