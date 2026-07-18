# git-cdc sample configuration — include from your global gitconfig:
#
#   git config --global include.path /path/to/.gitconfig.cdc
#
# or copy the sections you need. Keys under [cdc] are read by git-cdc;
# repo-local values (plain `git config` inside a repo) override these.

# The filter driver — safe to keep global, it only activates for paths
# tracked in a repo's .gitattributes (`git cdc track '*.bin'`).
[filter "cdc"]
	clean = git-cdc clean
	smudge = git-cdc smudge

# --- Remote: pick ONE of the two modes ---------------------------------

# Mode 1: git-cdc-server (central auth, batch API)
# [cdc]
# 	url = http://your-server:8077
# 	token = your-secret-token

# Mode 2: serverless — CLI talks straight to an S3-compatible bucket.
# Credentials come from the AWS chain (env vars, ~/.aws, IMDS), never
# from gitconfig. If cdc.s3.bucket is set it wins over cdc.url.
# [cdc "s3"]
# 	bucket = git-cdc
# 	prefix = chunks/
# 	endpoint = http://127.0.0.1:9000     ; MinIO/RustFS/R2 only — omit for AWS
# 	force-path-style = true              ; MinIO only

# --- Chunking (optional) -----------------------------------------------
# Defaults: min 512 KiB, avg 2 MiB, max 8 MiB. Prefer setting these
# repo-locally: every client of a repo should chunk with the same values,
# or identical content re-cleans into different (valid) manifests.
# [cdc "chunk"]
# 	min = 64k      ; 64 B – 1 MiB
# 	avg = 256k     ; 256 B – 4 MiB
# 	max = 1m       ; 1 KiB – 16 MiB, and min <= avg <= max
