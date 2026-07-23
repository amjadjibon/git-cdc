# git-cdc sample configuration — include from your global gitconfig:
#
#   git config --global include.path /path/to/.gitconfig.cdc
#
# or copy the sections you need. Keys under [cdc] are read by git-cdc;
# repo-local values (plain `git config` inside a repo) override these.

# The filter driver — safe to keep global, it only activates for paths
# tracked in a repo's .gitattributes (`git cdc track '*.bin'`).
[filter "cdc"]
	process = git-cdc filter-process
	clean = git-cdc clean
	smudge = git-cdc smudge

# --- Remote: pick ONE of the modes below --------------------------------

# Mode 1: git-cdc-server (central auth, batch API)
# [cdc]
# 	url = http://your-server:8077
# 	token = your-secret-token

# Mode 2: serverless — CLI talks straight to any OpenDAL service (s3,
# azblob, gcs, dropbox, b2, sftp, ftp, gdrive, swift, webdav, onedrive).
# Credentials come from each service's own standard chain (for S3: env
# vars, ~/.aws, IMDS), never from gitconfig. If cdc.opendal.scheme is set
# it wins over cdc.url. `option` may repeat, one KEY=VALUE pair per line.
#
# Real AWS S3 — region and enable_virtual_host_style are required; OpenDAL
# has no built-in default for either (unlike the old cdc.s3.* flags).
# [cdc "opendal"]
# 	scheme = s3
# 	option = bucket=git-cdc
# 	option = region=us-east-1
# 	option = enable_virtual_host_style=true
# 	prefix = chunks/
#
# MinIO / RustFS / R2 (path-style, custom endpoint):
# [cdc "opendal"]
# 	scheme = s3
# 	option = bucket=git-cdc
# 	option = region=us-east-1
# 	option = endpoint=http://127.0.0.1:9000
# 	prefix = chunks/

# Mode 3: SSH — chunks on any host you can ssh into (git-cdc must be
# installed there). Access control is your ssh keys + file permissions.
# [cdc "ssh"]
# 	remote = user@host
# 	path = /srv/cdc-chunks

# --- Chunking (optional) -----------------------------------------------
# Defaults: min 512 KiB, avg 2 MiB, max 8 MiB. Prefer setting these
# repo-locally: every client of a repo should chunk with the same values,
# or identical content re-cleans into different (valid) manifests.
# [cdc "chunk"]
# 	min = 64k      ; 64 B – 1 MiB
# 	avg = 256k     ; 256 B – 4 MiB
# 	max = 1m       ; 1 KiB – 16 MiB, and min <= avg <= max
