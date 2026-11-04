#!/bin/sh
# Rebuild the Java reflection bridge that the embedded JVM loads.
set -e
cd "$(dirname "$0")/.."
javac -d crates/bsh-jvm/java-classes crates/bsh-jvm/java/bshrs/Bridge.java
echo "bridge rebuilt"
