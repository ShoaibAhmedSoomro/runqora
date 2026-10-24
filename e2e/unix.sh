#!/usr/bin/env bash
# Runqora end-to-end check for Linux and macOS, driven through the `rq` command line against a real install in a temporary
# folder, on spare ports. Usage: e2e/unix.sh <rq binary> <manifest.json> [modules to install, default: apache php mariadb phpmyadmin]
# The manifest decides where the programs come from (CI points it at the freshly built stack bundles).
set -u
RQ=$1; export RUNQORA_MANIFEST=$2; shift 2
MODULES=${*:-apache php mariadb phpmyadmin}
export RUNQORA_DIR; RUNQORA_DIR=$(mktemp -d)
export RUNQORA_PORT_APACHE=18080 RUNQORA_PORT_PHP=19074 RUNQORA_PORT_MARIADB=13306
pass=0; fail=0
check() { # check "what" command...
  local what=$1; shift
  if "$@" >/tmp/rq-check.out 2>&1; then pass=$((pass + 1)); echo "  pass  $what"; else fail=$((fail + 1)); echo "  FAIL  $what"; sed 's/^/        /' /tmp/rq-check.out | tail -15; fi
}
listening() { (exec 3<>/dev/tcp/127.0.0.1/$1) 2>/dev/null; }
page() { curl -fsS --max-time 60 "$@"; }
trap '"$RQ" stop >/dev/null 2>&1; rm -rf "$RUNQORA_DIR"' EXIT

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
check "doctor finds nothing broken" bash -c "'$RQ' doctor | grep -vi 'problem' >/dev/null"

echo; echo "passed $pass, failed $fail"
exit "$fail"
