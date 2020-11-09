# Xenon

Xenon is a WebDriver proxy, for running multiple WebDriver sessions through a single hub.

This makes it effectively a drop-in alternative to Selenium Server (including standalone or grid).

## Purpose

To build a leaner, more efficient tool for managing multiple browsers, WebDriver
instances, and WebDriver sessions, that uses minimal system resources (CPU and memory)
and stays out of the way. Your test should operate exactly as if you were
pointing directly at a single webdriver instance running on a specific port,
without the complexity of having to manage either a Selenium server or
multiple WebDriver instances.

## Why not just use Selenium Server?

Selenium server is a fantastic tool and performs its job very well, however
it is written in Java and consumes a lot of system resources.

By contrast, Xenon is written in Rust and is extremely fast and light-weight.
It is built on top of async-await, tokio, and hyper.

## Status

Xenon currently works as a drop-in replacement for Selenium Standalone in most cases,
although 100% feature parity with Selenium is not a goal of Xenon.

Grid-like functionality is also supported (see "Running multiple nodes" below).

Xenon is able to run the full [thirtyfour](https://github.com/stevepryde/thirtyfour)
(Rust WebDriver client) test suite using 10 Chrome instances concurrently,
with no modification required to the test code.

## Getting Started

### Set up configuration in xenon.yml

First, set up the YAML config file (xenon.yml) for example like this:

    ---
    browsers:
      - name: chrome
        driver_path: /usr/local/bin/chromedriver
        sessions_per_driver: 1
        max_sessions: 10
    ports:
      - "40001-41000"

This tells it to start /usr/local/bin/chromedriver for any new session where
browserName is `chrome`. We will start a new chromedriver instance for every
session. No more than 10 sessions can be active at any one time.
The port range defines the ports that can be used for chromedriver.

You can add additional browsers each with different session limits.
You can even add multiple chromedriver configs as long as each one has a
different `name` (this will match against the `browserName` setting of your
desired capabilities arguments in your WebDriver client).

### Download and install Xenon

You can install the binary directly from crates.io like this:

    cargo install xenon-webdriver

This will be installed for the current user.

Alternatively you can build from source by cloning this repo and running:

    cargo build --release

If building from source, the binary will be at `./target/release/xenon-webdriver`.

### Run Xenon

Now you can just start Xenon with no arguments. This assumes you have the
`xenon-webdriver` binary in the same directory as `xenon.yml`.

    ./xenon-webdriver

You should see something like this:

    [2020-05-23T13:55:34Z DEBUG xenon::server] Config loaded:
        XenonConfig {
            browsers: [
                BrowserConfig {
                    name: "chrome",
                    version: None,
                    os: None,
                    driver_path: "/usr/local/bin/chromedriver",
                    sessions_per_driver: 1,
                    max_sessions: 10,
                },
            ],
            ports: [
                "40001-41000",
            ],
        }
    [2020-05-23T13:55:34Z INFO  xenon::server] Server running at 127.0.0.1:4444

You can now run your selenium/WebDriver tests and point them at 127.0.0.1:4444
just as you normally would. Xenon also optionally supports running at
127.0.0.1:4444/wd/hub for compatibility with tests that are set up to use selenium hub.

### Running multiple nodes (i.e. Grid functionality)

Each Xenon server can act as a hub, node, or standalone server (or all of these at once).
Another way to say this is that each Xenon server can support local browsers as well as
defer to remote nodes (other Xenon servers) that can provide additional browsers.
To use a Xenon server as a node, we just add that server's URL under the `nodes` section
in the config for the server that will act as the hub, like this:

"Hub" server configuration:

    ---
    nodes:
      - name: node1
        url: localhost:8888

NOTE: The hub could also specify `browsers:` and `ports:` if you want to also run
local browsers off the same hub.

The "node" server configuration is the same as the standalone configuration (see above).

However, this hub configuration assumes the node will be running on port 8888, so you
would start the node like this:

    ./xenon-webdriver --port 8888

The node does not actually know it is serving requests from another Xenon server. Since
Xenon behaves as a WebDriver proxy, we can just forward requests to any other Xenon
server and it "just works". There is one piece of information we need from the
"node" and that is the list of browsers it provides in its configuration.
This is requested by the hub automatically when it first starts up.
The hub will poll the `/node/config` endpoint of each node every 60 seconds until a
successful response is received. This allows the servers to be started in any order.

In summary, each Xenon server can provide local or remote browsers, or both. A "local"
browser is where this server takes care of starting each WebDriver instance
(chromedriver, geckodriver etc) and talks to it directly. A "remote" browser is just a
"local" browser running on another Xenon server.

### Running under Xvfb for "headless" operation (Linux only)

You can run Xenon under Xvfb which creates a new X server and runs the browser
sessions there instead, so that you don't end up with your mouse and keyboard
inputs interfering with tests.

To do this, just install Xvfb using your distro's package manager and then run:

    xvfb-run --server-args="-screen 0 1024x768x24" ./xenon

#### VNC output

If you run Xvfb (as above) you can also get a live view by running a VNC
server on the Xvfb display.

https://stackoverflow.com/questions/12050021/how-to-make-xvfb-display-visible

## Planned features

- Support for forwarding requests from one Xenon server to another, including across a network.
- Docker and Docker Compose

## LICENSE

This work is licensed under MIT.

`SPDX-License-Identifier: MIT`
