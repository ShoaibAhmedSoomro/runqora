#!/usr/bin/env bash
# PHP for Linux and macOS: the static-php-cli builds of php-fpm and php (cli) with the "bulk" extension set (no system
# libraries needed), plus the php.ini template Runqora turns into php.ini. usage: build-php.sh <platform> <static-php os-arch>
. "$(dirname "$0")/lib.sh"
PLAT=$1; SP=$2
VER=8.3.32
W=$PWD/work-php; rm -rf "$W"; mkdir -p "$W/out" dist; cd "$W"
base=https://dl.static-php.dev/static-php-cli/bulk
fetch $base/php-$VER-fpm-$SP.tar.gz fpm.tgz ""
fetch $base/php-$VER-cli-$SP.tar.gz cli.tgz ""
fetch https://raw.githubusercontent.com/php/php-src/php-$VER/php.ini-development out/php.ini-development ""
fetch https://raw.githubusercontent.com/php/php-src/php-$VER/php.ini-production out/php.ini-production ""
tar -C out -xzf fpm.tgz; tar -C out -xzf cli.tgz
chmod +x out/php out/php-fpm
echo "$VER" > out/.bundle-version
./out/php -v | head -1
./out/php -m | tr '\n' ' '; echo
tar -C out -czf "../dist/php-$VER-$PLAT.tar.gz" .
sha256 "../dist/php-$VER-$PLAT.tar.gz"
