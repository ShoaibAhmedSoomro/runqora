#!/usr/bin/env bash
# Build a relocatable Apache httpd (event MPM, mod_ssl, mod_proxy_fcgi, mod_rewrite ...) for Linux or macOS.
# usage: build-apache.sh <platform name for the archive>   (output: dist/apache-<version>-<platform>.tar.gz)
. "$(dirname "$0")/lib.sh"
PLAT=$1
HTTPD=2.4.66; APR=1.7.6; APU=1.6.3; PCRE=10.45; SSL=3.0.18
W=$PWD/work-apache; DEPS=$W/deps; OUT=$W/out; rm -rf "$W"; mkdir -p "$W" "$DEPS" dist; cd "$W"

fetch https://github.com/openssl/openssl/releases/download/openssl-$SSL/openssl-$SSL.tar.gz ssl.tgz d80c34f5cf902dccf1f1b5df5ebb86d0392e37049e5d73df1b3abae72e4ffe8b
fetch https://github.com/PCRE2Project/pcre2/releases/download/pcre2-$PCRE/pcre2-$PCRE.tar.bz2 pcre.tbz ""
fetch https://archive.apache.org/dist/apr/apr-$APR.tar.bz2 apr.tbz 49030d92d2575da735791b496dc322f3ce5cff9494779ba8cc28c7f46c5deb32
fetch https://archive.apache.org/dist/apr/apr-util-$APU.tar.bz2 apu.tbz a41076e3710746326c3945042994ad9a4fcac0ce0277dd8fea076fec3c9772b5
fetch https://archive.apache.org/dist/httpd/httpd-$HTTPD.tar.bz2 httpd.tbz 94d7ff2b42acbb828e870ba29e4cbad48e558a79c623ad3596e4116efcfea25a
for f in ssl.tgz pcre.tbz apr.tbz apu.tbz httpd.tbz; do tar xf $f; done

export CFLAGS="-O2 -fPIC"
( cd openssl-$SSL && ./config --prefix="$DEPS" --libdir=lib no-shared no-tests no-docs >/dev/null && make -j"$NPROC" >/dev/null && make install_sw >/dev/null )
( cd pcre2-$PCRE && ./configure --prefix="$DEPS" --disable-shared --enable-static >/dev/null && make -j"$NPROC" >/dev/null && make install >/dev/null )
( cd apr-$APR && ./configure --prefix="$DEPS" --disable-shared >/dev/null && make -j"$NPROC" >/dev/null && make install >/dev/null )
( cd apr-util-$APU && ./configure --prefix="$DEPS" --with-apr="$DEPS" --disable-shared --with-crypto --with-openssl="$DEPS" --with-expat=/usr >/dev/null 2>&1 || ./configure --prefix="$DEPS" --with-apr="$DEPS" --disable-shared --with-crypto --with-openssl="$DEPS" >/dev/null && make -j"$NPROC" >/dev/null && make install >/dev/null )

# the prefix is a marker: Runqora rewrites it (ServerRoot) when it installs and starts with `httpd -d <folder>`
PREFIX=/__RUNQORA__/apache
( cd httpd-$HTTPD && ./configure --prefix=$PREFIX --with-mpm=event --enable-so --enable-mods-shared=most --enable-ssl --with-ssl="$DEPS" \
    --enable-proxy --enable-proxy-fcgi --enable-rewrite --enable-headers --enable-deflate --enable-expires --enable-http2=no \
    --with-apr="$DEPS" --with-apr-util="$DEPS" --with-pcre="$DEPS/bin/pcre2-config" --disable-lua --disable-luajit >/dev/null \
  && make -j"$NPROC" >/dev/null && make install DESTDIR="$OUT" >/dev/null )

A=$OUT$PREFIX
cp "$DEPS/bin/openssl" "$A/bin/openssl"          # Runqora makes its local HTTPS certificates with it
rm -rf "$A/manual" "$A/cgi-bin" "$A/include" "$A/build" "$A/error/include"
mkdir -p "$A/logs" "$A/htdocs"
echo "$HTTPD" > "$A/.bundle-version"
"$A/bin/httpd" -v | head -1
( cd "$A/modules" && ls | grep -E 'mod_(ssl|proxy_fcgi|rewrite|headers)\.so' ) | sort | tr '\n' ' '; echo
tar -C "$A" -czf "$W/../dist/apache-$HTTPD-$PLAT.tar.gz" .
sha256 "$W/../dist/apache-$HTTPD-$PLAT.tar.gz"
