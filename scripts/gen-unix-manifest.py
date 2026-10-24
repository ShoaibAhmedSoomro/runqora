"""Writes app/src-tauri/manifest.unix.json: the Linux and macOS download list, built from the Windows manifest (versions),
the publishers' own checksums, and Runqora's own stack builds (stack.json from the Stack workflow).

  python scripts/gen-unix-manifest.py                      refresh the file in the repository
  python scripts/gen-unix-manifest.py --stack stack.json --url-base http://127.0.0.1:8765 --out m.json
                                                           a test manifest that takes the stack bundles from a local web server
Needs the GitHub CLI (gh) for the release checksums.
"""
import argparse, copy, json, pathlib, subprocess, urllib.request

ap = argparse.ArgumentParser()
ap.add_argument("--stack", help="stack.json from the Stack workflow (default: app/src-tauri/stack.json)")
ap.add_argument("--url-base", help="serve the stack bundles from this base URL instead of the stack release")
ap.add_argument("--out", help="where to write (default: app/src-tauri/manifest.unix.json)")
args = ap.parse_args()

root = pathlib.Path(__file__).resolve().parent.parent / "app" / "src-tauri"
win = json.load(open(root / "manifest.json"))
W = {c["id"]: c for c in win["components"]}
PLAT = ["linux-x86_64", "linux-aarch64", "macos-aarch64"]


def get(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "runqora"}), timeout=60).read().decode()


def gh_digests(repo, tag):
    out = subprocess.run(["gh", "api", f"repos/{repo}/releases/tags/{tag}", "-q", '.assets[] | .name + " " + .digest'], capture_output=True, text=True, check=True).stdout
    return {l.split()[0]: l.split()[1].replace("sha256:", "") for l in out.splitlines() if " sha256:" in l}


def base(id, **over):
    c = copy.deepcopy(W[id])
    c.update(over)
    return c


comps = []

# pure PHP applications: the same files everywhere
for id in ["phpmyadmin", "wordpress", "composer", "adminer"]:
    comps.append(copy.deepcopy(W[id]))

# Apache, PHP-FPM and MariaDB have no portable download for these systems: Runqora builds them (stack.yml)
STACK = "https://github.com/ShoaibAhmedSoomro/runqora/releases/download/stack-{tag}/{name}-{ver}-{plat}.tar.gz"
stack_file = pathlib.Path(args.stack) if args.stack else root / "stack.json"
stack = json.load(open(stack_file)) if stack_file.exists() else {}

# the version a stack bundle carries is the one the build scripts name
VERSIONS = {"apache": "2.4.66", "php": "8.3.32", "mariadb": "10.11.19"}


def built(id, name, **fields):
    ver = VERSIONS[id]
    c = dict(id=id, name=name, version=ver, url="", sha256="", strip_prefix="", **fields)
    c["variants"] = {}
    for p in PLAT:
        h = stack.get(f"{id}-{p}")
        if h:
            url = STACK.format(tag=stack["tag"], name=id, ver=ver, plat=p)
            if args.url_base:
                url = f"{args.url_base}/{id}-{ver}-{p}.tar.gz"
            c["variants"][p] = {"url": url, "sha256": h, "strip_prefix": ""}
    return c


apache = built("apache", "Apache HTTP Server", extract_to="apache", binary="apache/bin/httpd", args=["-d", "{root}/apache"],
               pidfile="apache/logs/httpd.pid", port=8000, admin_url="http://localhost:{port}/", configs=W["apache"]["configs"], logs=W["apache"]["logs"])
php = built("php", "PHP (FPM)", extract_to="php", binary="php/php-fpm", args=["--nodaemonize", "--fpm-config", "{root}/php/php-fpm.conf", "-c", "{root}/php/php.ini"],
            pidfile="php/php-fpm.pid", port=9074, configs=["php/php.ini", "php/php-fpm.conf"], logs=["php/php-fpm.log"])
maria = built("mariadb", "MariaDB", extract_to="mariadb", binary="mariadb/bin/mariadbd", args=["--defaults-file={root}/mariadb/my.ini", "--standalone"],
              pidfile="mariadb/data/mysql.pid", port=3306, admin_url=W["mariadb"]["admin_url"], configs=["mariadb/my.ini"], logs=["mariadb/data/mysql_error.log"])
comps = [c for c in (apache, php, maria) if c["variants"]] + comps  # hidden until the stack workflow has published them

# Java (Temurin): official tarballs
jtag = "jdk-21.0.12.1+1"
jd = gh_digests("adoptium/temurin21-binaries", jtag)
jre = base("jre", binary="jre/bin/java", variants={}, url="", sha256="")
for p, osn, arch in [("linux-x86_64", "linux", "x64"), ("linux-aarch64", "linux", "aarch64"), ("macos-aarch64", "mac", "aarch64")]:
    name = f"OpenJDK21U-jre_{arch}_{osn}_hotspot_21.0.12.1_1.tar.gz"
    jre["variants"][p] = {"url": f"https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/{name}", "sha256": jd[name],
                          "strip_prefix": "*" if osn == "linux" else f"{jtag}-jre/Contents/Home/"}
comps.append(jre)

# Tomcat: one tar.gz for every system; it runs on the Java above
tv = W["tomcat"]["version"]
turl = f"https://archive.apache.org/dist/tomcat/tomcat-{tv.split('.')[0]}/v{tv}/bin/apache-tomcat-{tv}.tar.gz"
tom = base("tomcat", url=turl, strip_prefix=f"apache-tomcat-{tv}/", exe="jre/bin/java", sha256="")
tom["sha512"] = get(turl + ".sha512").split()[0]
tom["args"] = [a.replace("bootstrap.jar;", "bootstrap.jar:") for a in tom["args"]]  # the class path separator
comps.append(tom)

# Mailpit: official tarballs
mv = W["mailpit"]["version"]
md = gh_digests("axllent/mailpit", f"v{mv}")
mp = base("mailpit", binary="mailpit/mailpit", variants={}, url="", sha256="")
for p, a in [("linux-x86_64", "linux-amd64"), ("linux-aarch64", "linux-arm64"), ("macos-aarch64", "darwin-arm64")]:
    n = f"mailpit-{a}.tar.gz"
    mp["variants"][p] = {"url": f"https://github.com/axllent/mailpit/releases/download/v{mv}/{n}", "sha256": md[n], "strip_prefix": ""}
comps.append(mp)

# SFTPGo: official tar.xz (the FTP port is above 1024: ports below need root here)
sv = W["sftpgo"]["version"]
sd = gh_digests("drakkan/sftpgo", f"v{sv}")
sf = base("sftpgo", binary="sftpgo/sftpgo", variants={}, url="", sha256="")
sf["env"] = dict(sf["env"], SFTPGO_FTPD__BINDINGS__0__PORT="2121")
for p, a in [("linux-x86_64", "linux_x86_64"), ("linux-aarch64", "linux_arm64"), ("macos-aarch64", "macOS_arm64")]:
    n = f"sftpgo_v{sv}_{a}.tar.xz"
    sf["variants"][p] = {"url": f"https://github.com/drakkan/sftpgo/releases/download/v{sv}/{n}", "sha256": sd[n], "strip_prefix": "*"}
comps.append(sf)

# Node.js: official tarballs
nv = W["node"]["version"]
sums = {l.split()[1]: l.split()[0] for l in get(f"https://nodejs.org/dist/v{nv}/SHASUMS256.txt").splitlines() if l.strip()}
nd = base("node", binary="node/bin/node", variants={}, url="", sha256="")
for p, a in [("linux-x86_64", "linux-x64"), ("linux-aarch64", "linux-arm64"), ("macos-aarch64", "darwin-arm64")]:
    n = f"node-v{nv}-{a}.tar.gz"
    nd["variants"][p] = {"url": f"https://nodejs.org/dist/v{nv}/{n}", "sha256": sums[n], "strip_prefix": "*"}
comps.append(nd)

for c in comps:
    if not c.get("sha1"):
        c.pop("sha1", None)
order = [c["id"] for c in win["components"]]
comps.sort(key=lambda c: order.index(c["id"]))
json.dump({"schema": win["schema"], "components": comps}, open(args.out or root / "manifest.unix.json", "w"), indent=2)
print("components:", [c["id"] for c in comps])
