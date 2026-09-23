# Security

Guixvis is a local package explorer. Its HTTP API reads package metadata and
graphs; it has no endpoint for installation, removal, shell execution, or
changing profiles. Copying a displayed command does not run it.

## Boundaries

The server binds to `127.0.0.1` and checks both the peer address and the HTTP
Host header. Browser requests with an Origin must use the server's HTTP origin,
including its host and port. Malformed or duplicate Host/Origin headers and
cross-site fetches are rejected. Direct local clients can omit Origin. Security
headers, including the content security policy, apply to rejected responses
as well as successful ones.

Search length, result count, graph depth, graph size, request bodies, and
expensive concurrent requests are bounded. Package metadata is rendered as
text. Homepage launchers allow HTTP(S) links. Command-copy helpers validate
package names and quote arguments; they do not interpolate metadata into a
shell that Guixvis executes.

Cache writes use a newly created private temporary file and an atomic rename.
Existing temporary-file paths are never opened with truncation. Snapshot reads
have a size limit; malformed snapshots are rejected. The `guix describe`
subprocess has a timeout and bounded captured output. Guile indexing runs the
user's locally installed Guix, so installed channels remain trusted code.

## Limits

There is no login or protection from a process already running as your user.
Do not expose the service through a public reverse proxy or bind it through
a tunnel for untrusted users. Loopback checks are not a replacement for
authentication on a shared network service. Local package metadata may reveal
which channels you use.

The Emacs client is restricted to loopback service addresses. It uses bundled
HTTP and JSON libraries, validates copied commands, and keeps package actions
under your control. Review copied commands, especially removals, before running
them in the intended profile.

## Reporting a problem

For a reproducible defect, include the Guixvis version, interface used, and a
small example. Remove tokens, private channel URLs, and personal paths from
logs. For a vulnerability that should not be public yet, use the repository
host's private vulnerability reporting option when available; otherwise contact
the maintainer privately before posting exploit details.

Maintainers should run `cargo audit` against `Cargo.lock`, review any finding
for reachability, and rerun the relevant regression tests before publishing.
