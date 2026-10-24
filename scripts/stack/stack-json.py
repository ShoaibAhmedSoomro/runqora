"""stack.json for a folder of bundles named <program>-<version>-<platform>.tar.gz: {"tag": ..., "<program>-<platform>": sha256}."""
import glob, hashlib, json, os, sys

folder, tag = sys.argv[1], sys.argv[2]
out = {"tag": tag}
for f in sorted(glob.glob(os.path.join(folder, "*.tar.gz"))):
    name = os.path.basename(f)[: -len(".tar.gz")]
    prog, rest = name.split("-", 1)  # apache-2.4.66-linux-x86_64
    platform = "-".join(rest.split("-")[1:])
    out[f"{prog}-{platform}"] = hashlib.sha256(open(f, "rb").read()).hexdigest()
json.dump(out, open(os.path.join(folder, "stack.json"), "w"), indent=1)
print(json.dumps(out, indent=1))
