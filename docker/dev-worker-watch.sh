#!/bin/sh
set -eu
cd /src
/docker/reload-worker-if-built.sh
exec cargo watch -q -w src -w migrations -s /docker/reload-worker-if-built.sh
