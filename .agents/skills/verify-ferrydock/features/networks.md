# Networks

Entry points: **Networks** sidebar item; a container's **More → View networks**
or right-click menu; **All networks** clears the container scope. Search matches
name, ID, driver, Compose project and subnet. Ctrl+F focuses search, Ctrl+R
refreshes, and Up/Down selects a network when the workspace has focus. Select
a network to read subnets/gateways, internal/IPv6 flags and configured endpoints.
Click a local container name to open its existing details. **Copy** copies full
addresses, aliases/DNS names and any endpoint error; hover reveals full values.

Run `python3 scripts/verify-headless.py --flow networks`. Require a passed
`result.json`, owned process exit, and inspect the endpoint, scoped-container,
large-inventory, large-endpoint, Light and disconnected/reconnected PNGs.
The GPUI input flow covers sidebar/session preservation, keyboard search and
selection, configured stopped endpoints without an active address, virtual
clipboard contents, failure/retry, obsolete-result rejection, opening a stopped
container, container scoping, and virtualization of 10,000 networks / 2,000
endpoints. It uses fixture worker events and the real WGPU view. It does not
prove compositor clipboard integration, native desktop input, or real Docker.

Run `bash scripts/verify-networks.sh` for production worker/daemon proof. It pins
the local Unix endpoint, creates two uniquely labelled bridge networks and two
Alpine containers, stops one, and verifies configured membership, IPv4 CIDR,
subnet/gateway and aliases from inspect, stopped aliases without an active IP,
an empty network and membership after disconnecting its owned running endpoint.
The fixture has no ports, bind mounts or volumes. Cleanup removes only containers
and networks carrying the run's ownership label. Require
`.verification/networks/<run>/result.json` with `status: passed` and
`cleanup_success: true`. On failure retain logs and independently verify cleanup.

IPv6 rendering uses fixture data; real daemon IPv6, host/none/shared network
namespaces, remote Swarm endpoints, physical desktop interactions and DNS or
connectivity tests remain outside these proofs. The product is read-only.
