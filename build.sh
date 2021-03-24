#!/bin/bash
mkdir graphql_cache
mv graphql_cache.tar.gz graphql_cache
cd graphql_cache
tar xzf graphql_cache.tar.gz
cargo update
cross build --target arm-unknown-linux-musleabi --release
