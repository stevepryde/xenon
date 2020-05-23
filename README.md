# Xenon

Xenon is a WebDriver proxy, for running multiple WebDriver sessions through a single hub.

This makes it effectively a drop-in alternative to Selenium Server (Hub/Standalone).

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

This project is still in early stages, however it is already functional as a
drop-in replacement for Selenium Standalone. Grid-like functionality is planned
for a future release.

As of v0.2.0 Xenon is able to run the full [thirtyfour](https://github.com/stevepryde/thirtyfour)
(Rust WebDriver client) test suite using 10 Chrome instances concurrently,
with no modification required to the test code.

There is also a known issue with Xenon's error reporting. Currently Xenon will
report all upstream errors "as is", which works fine, but any errors handled
internally by Xenon will be reported to the client in a custom format, which
the selenium/WebDriver client very likely will not understand. I plan to
remedy this by coercing Xenon error codes and messages into a W3C WebDriver
compatible error format (for example "session not found" etc).

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

### Run Xenon

Now you can just start Xenon with no arguments. This assumes you have already
compiled Xenon and have the `xenon` binary in the same directory as `xenon.yml`.

    ./xenon

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
just as you normally would. Xenon also optionally supports running at the path
/wd/hub for compatibility with tests that are set up to use selenium hub.
