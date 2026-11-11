#!/bin/sh
set -eu
cd /src
/docker/reload-api-if-built.sh
exec cargo watch -q -w src -w migrations -s /docker/reload-api-if-built.sh
