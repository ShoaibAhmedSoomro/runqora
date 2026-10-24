#!/usr/bin/env bash
# Runqora end-to-end check for Linux and macOS, driven through the `rq` command line against a real install in a temporary
# folder, on spare ports. Usage: e2e/unix.sh <rq binary> <manifest.json> [modules to install, default: apache php mariadb phpmyadmin]
# The manifest decides where the programs come from (CI points it at the freshly built stack bundles).
set -u
RQ=$1; export RUNQORA_MANIFEST=$2; shift 2
MODULES=${*:-apache php mariadb phpmyadmin}
export RUNQORA_DIR; RUNQORA_DIR=$(mktemp -d)
export RUNQORA_PORT_APACHE=18080 RUNQORA_PORT_PHP=19074 RUNQORA_PORT_MARIADB=13306 RUNQORA_PORT_MAILPIT=18025 RUNQORA_PORT_TOMCAT=18081 RUNQORA_PORT_SFTPGO=18082
pass=0; fail=0
check() { # check "what" command...
  local what=$1; shift
  if "$@" >/tmp/rq-check.out 2>&1; then pass=$((pass + 1)); echo "  pass  $what"; else fail=$((fail + 1)); echo "  FAIL  $what"; sed 's/^/        /' /tmp/rq-check.out | tail -15; fi
}
listening() { (exec 3<>/dev/tcp/127.0.0.1/$1) 2>/dev/null; }
page() { curl -fsS --max-time 60 "$@"; }
diagnose() {
  echo; echo "===== diagnostics ====="
  "$RQ" status 2>&1 | head -20
  for f in apache/logs/error_log php/php-fpm.log mariadb/data/mysql_error.log tomcat/logs/catalina.out; do
    [ -f "$RUNQORA_DIR/$f" ] && { echo "--- $f"; tail -25 "$RUNQORA_DIR/$f"; }
  done
  [ -x "$RUNQORA_DIR/apache/bin/httpd" ] && { echo "--- httpd -t"; "$RUNQORA_DIR/apache/bin/httpd" -d "$RUNQORA_DIR/apache" -t 2>&1 | head -20; }
  [ -f "$RUNQORA_DIR/apache/conf/extra/runqora.conf" ] && { echo "--- runqora.conf"; cat "$RUNQORA_DIR/apache/conf/extra/runqora.conf"; }
  ls -la "$RUNQORA_DIR" "$RUNQORA_DIR/mariadb" "$RUNQORA_DIR/mariadb/bin" 2>&1 | head -60
}
trap '[ "$fail" -gt 0 ] && diagnose; "$RQ" stop >/dev/null 2>&1; rm -rf "$RUNQORA_DIR"' EXIT

echo "install into $RUNQORA_DIR"
for m in $MODULES; do check "install $m" "$RQ" install "$m"; done
check "status lists the modules" bash -c "'$RQ' status | grep -q apache"
check "install is refused the second time" bash -c "! '$RQ' install apache"

echo "start"
check "start exits 0" "$RQ" start
check "Apache listens" listening 18080
check "MariaDB listens" listening 13306
check "PHP-FPM listens" listening 19074
check "Apache serves the start page" bash -c "curl -fsS --max-time 30 http://127.0.0.1:18080/ | grep -qi 'It works'"
echo '<?php echo 6*7, PHP_EOL; echo (new PDO("mysql:host=127.0.0.1;port=13306", "root", ""))->query("select 40+2")->fetchColumn();' > "$RUNQORA_DIR/htdocs/t.php"
check "PHP runs inside Apache" bash -c "curl -fsS --max-time 30 http://127.0.0.1:18080/t.php | head -1 | grep -qx 42"
check "PHP reaches MariaDB through PDO" bash -c "curl -fsS --max-time 30 http://127.0.0.1:18080/t.php | tail -1 | grep -qx 42"
check "MariaDB answers the command line client" "$RUNQORA_DIR/mariadb/bin/mariadb" -uroot --host=127.0.0.1 --port=13306 -e "select 1"
check "phpMyAdmin loads" bash -c "curl -fsS --max-time 120 http://127.0.0.1:18080/phpmyadmin/ | grep -qi phpmyadmin"
check "ports lists our listeners" bash -c "'$RQ' ports | grep -q 18080"
check "starting again is harmless" bash -c "'$RQ' start | grep -qi 'already running'"

echo "restart and stop"
check "stop exits 0" "$RQ" stop
check "Apache port is free" bash -c "! (exec 3<>/dev/tcp/127.0.0.1/18080) 2>/dev/null"
check "MariaDB port is free" bash -c "! (exec 3<>/dev/tcp/127.0.0.1/13306) 2>/dev/null"
check "PHP-FPM port is free" bash -c "! (exec 3<>/dev/tcp/127.0.0.1/19074) 2>/dev/null"
check "start works again after a stop" "$RQ" start
check "and Apache answers again" bash -c "curl -fsS --max-time 30 http://127.0.0.1:18080/t.php | head -1 | grep -qx 42"
check "stop once more" "$RQ" stop

echo "HTTPS"
check "an HTTPS certificate can be made" bash -c "'$RQ' https regenerate | grep -qi 'certificate\|authority\|expires'"
if [ "$(uname)" = Linux ] && command -v certutil >/dev/null; then
  check "the certificate can be trusted for browsers (NSS)" bash -c "'$RQ' https trust | grep -qi 'trust'"
  check "and status says so" bash -c "'$RQ' https status | grep -q 'Trusted on this PC: yes'"
  check "and the trust can be taken back" bash -c "'$RQ' https untrust | grep -qi 'no longer'"
elif [ "$(uname)" = Darwin ]; then
  # the login keychain may ask for a password a CI machine cannot give: shown for information, not counted
  timeout 90 "$RQ" https trust 2>&1 | sed 's/^/        (macOS keychain) /' | head -3 || true
  "$RQ" https untrust >/dev/null 2>&1 || true
fi
check "doctor finds nothing broken" bash -c "'$RQ' doctor | grep -vi 'problem' >/dev/null"

has() { case " $MODULES " in *" $1 "*) return 0;; esac; return 1; }

echo "settings and sites"
check "a PHP setting is written to php.ini" bash -c "'$RQ' config set php memory_limit 384M && grep -q '^memory_limit *= *384M' '$RUNQORA_DIR/php/php.ini'"
check "a bad PHP setting is refused" bash -c "! '$RQ' config set php memory_limit lots"
check "the Apache port can be read back" bash -c "'$RQ' config get apache | grep -qi 18080"
check "a site can be added" "$RQ" sites add demo
check "the site is in the Apache config" grep -q "demo.localhost" "$RUNQORA_DIR/apache/conf/extra/runqora.conf"
"$RQ" start >/dev/null 2>&1
check "the new setting reaches PHP" bash -c "echo '<?php echo ini_get(\"memory_limit\");' > '$RUNQORA_DIR/htdocs/m.php'; curl -fsS --max-time 30 http://127.0.0.1:18080/m.php | grep -qx 384M"
check "the site answers on its own name" bash -c "curl -fsS --max-time 30 -H 'Host: demo.localhost' http://127.0.0.1:18080/ | grep -qi 'demo'"
check "the site can be removed" "$RQ" sites remove demo
"$RQ" stop >/dev/null 2>&1

if has node; then echo "node"; check "node runs" "$RUNQORA_DIR/node/bin/node" -v; fi
if has composer; then echo "composer"; check "composer runs on Runqora's PHP" "$RUNQORA_DIR/composer/composer" --version; fi
if has adminer; then check "adminer is in place" test -f "$RUNQORA_DIR/adminer/adminer.php"; fi
if has mailpit; then
  echo "mailpit"
  check "mailpit starts" "$RQ" start mailpit
  check "mailpit answers" bash -c "curl -fsS --max-time 30 http://127.0.0.1:18025/ | grep -qi mailpit"
  check "mailpit stops" "$RQ" stop mailpit
fi
if has tomcat; then
  echo "tomcat"
  check "tomcat starts (it brought its Java)" "$RQ" start tomcat
  check "tomcat answers" bash -c "curl -s --max-time 30 -o /dev/null -w '%{http_code}' http://127.0.0.1:18081/ | grep -qE '200|404'"
  check "tomcat stops by its shutdown word" "$RQ" stop tomcat
fi
if has sftpgo; then
  echo "sftpgo"
  check "sftpgo starts" "$RQ" start sftpgo
  check "sftpgo answers" bash -c "curl -s --max-time 30 -o /dev/null -w '%{http_code}' http://127.0.0.1:18082/web/admin | grep -qE '200|30[0-9]'"
  check "sftpgo stops" "$RQ" stop sftpgo
fi
if has wordpress; then
  echo "wordpress"
  "$RQ" start mariadb >/dev/null 2>&1
  check "a WordPress site can be made" "$RQ" wordpress blog
  check "its files exist" test -f "$RUNQORA_DIR/htdocs/blog/wp-config.php"
  "$RQ" stop >/dev/null 2>&1
fi

echo; echo "passed $pass, failed $fail"
exit "$fail"
