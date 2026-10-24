#!/usr/bin/env bash
# MariaDB server from source, relocatable (bundled SSL, zlib and pcre; no libaio), for Linux or macOS.
# usage: build-mariadb.sh <platform>    (output: dist/mariadb-<version>-<platform>.tar.gz)
. "$(dirname "$0")/lib.sh"
PLAT=$1
VER=10.11.19
W=$PWD/work-mariadb; rm -rf "$W"; mkdir -p "$W" dist; cd "$W"
fetch https://archive.mariadb.org/mariadb-$VER/source/mariadb-$VER.tar.gz src.tgz b8e543ee69d380fb1cfd563226f49e0fe96e4d67e7b7a9045ee514a168ed2066
tar xf src.tgz
PREFIX=/__RUNQORA__/mariadb
SSLOPT=bundled
if [ "$(uname)" = Linux ]; then
  # the client library of MariaDB wants GnuTLS when it is told "bundled" on Linux: give it a static OpenSSL instead
  fetch https://github.com/openssl/openssl/releases/download/openssl-3.0.18/openssl-3.0.18.tar.gz ssl.tgz d80c34f5cf902dccf1f1b5df5ebb86d0392e37049e5d73df1b3abae72e4ffe8b
  tar xf ssl.tgz
  ( cd openssl-3.0.18 && CFLAGS="-O2 -fPIC" ./config --prefix="$W/deps" --libdir=lib no-shared no-tests >/dev/null && make -j"$NPROC" >/dev/null && make install_sw >/dev/null )
  SSLOPT="$W/deps"
fi
cmake -S mariadb-$VER -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$PREFIX -DINSTALL_LAYOUT=STANDALONE \
  -DWITH_SSL=$SSLOPT -DWITH_ZLIB=bundled -DWITH_PCRE=bundled -DWITH_LIBAIO=OFF -DWITH_SYSTEMD=no -DWITH_UNIT_TESTS=OFF -DWITH_WSREP=OFF \
  -DPLUGIN_ROCKSDB=NO -DPLUGIN_MROONGA=NO -DPLUGIN_TOKUDB=NO -DPLUGIN_CONNECT=NO -DPLUGIN_SPIDER=NO -DPLUGIN_OQGRAPH=NO -DPLUGIN_SPHINX=NO \
  -DPLUGIN_COLUMNSTORE=NO -DPLUGIN_S3=NO -DPLUGIN_AUTH_PAM=NO -DPLUGIN_AUTH_GSSAPI=NO -DWITHOUT_SERVER=OFF -DWITH_EMBEDDED_SERVER=OFF \
  -DWITH_MARIABACKUP=OFF -DWITH_READLINE=OFF -DWITH_LIBFMT=bundled >/dev/null
cmake --build build -j"$NPROC"
DESTDIR=$W/out cmake --install build >/dev/null
M=$W/out$PREFIX
rm -rf "$M/mysql-test" "$M/sql-bench" "$M/include" "$M/man" "$M/docs"
echo "$VER" > "$M/.bundle-version"
"$M/bin/mariadbd" --version
tar -C "$M" -czf "$W/../dist/mariadb-$VER-$PLAT.tar.gz" .
sha256 "$W/../dist/mariadb-$VER-$PLAT.tar.gz"
