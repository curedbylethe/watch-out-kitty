#!/bin/sh
# Installs the wok and wok-daemon release binaries for this machine.
set -eu

usage() {
	printf 'usage: install.sh [-h|--help] [--dir <directory>]\n'
}

dir=
while [ $# -gt 0 ]; do
	case $1 in
	-h | --help)
		usage
		exit 0
		;;
	--dir)
		if [ $# -lt 2 ]; then
			printf 'install.sh: --dir needs a directory\n' >&2
			exit 1
		fi
		dir=$2
		shift 2
		;;
	*)
		printf 'install.sh: unknown argument: %s\n' "$1" >&2
		usage >&2
		exit 1
		;;
	esac
done

os=$(uname -s)
machine=$(uname -m)

case $os/$machine in
Darwin/arm64 | Darwin/aarch64) target=aarch64-apple-darwin ;;
Darwin/x86_64 | Darwin/amd64) target=x86_64-apple-darwin ;;
Linux/arm64 | Linux/aarch64) target=aarch64-unknown-linux-gnu ;;
Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-gnu ;;
*)
	printf 'install.sh: unsupported machine: uname -s reports "%s" and uname -m reports "%s"\n' "$os" "$machine" >&2
	printf 'install.sh: supported triples are aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu and x86_64-unknown-linux-gnu\n' >&2
	exit 1
	;;
esac

if [ -z "$dir" ]; then
	if [ -z "${HOME:-}" ]; then
		printf 'install.sh: HOME is empty; pass --dir <directory>\n' >&2
		exit 1
	fi
	dir=$HOME/.local/bin
fi

url="https://github.com/curedbylethe/watch-out-kitty/releases/latest/download/wok-$target.tar.gz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM
archive=$work/wok-$target.tar.gz

if ! curl -fsSL --proto '=https' -o "$archive" "$url"; then
	printf 'install.sh: no release archive was found for this target: %s\n' "$url" >&2
	exit 1
fi

mkdir "$work/extract"
tar -xzf "$archive" -C "$work/extract" || exit 1
for name in wok wok-daemon; do
	if [ ! -f "$work/extract/$name" ]; then
		printf 'install.sh: %s is missing from %s\n' "$name" "$url" >&2
		exit 1
	fi
done

mkdir -p "$dir"
for name in wok wok-daemon; do
	# A running executable cannot be overwritten in place on Linux (ETXTBSY).
	rm -f "$dir/$name"
	cp "$work/extract/$name" "$dir/$name"
	chmod 755 "$dir/$name"
done

line='export PATH="'"$dir"':$PATH"'

profile=
if [ -n "${HOME:-}" ]; then
	shell=${SHELL:-}
	case ${shell##*/} in
	zsh) profile="${ZDOTDIR:-$HOME}/.zshrc" ;;
	bash)
		if [ "$os" = Darwin ]; then
			profile=$HOME/.bash_profile
		else
			profile=$HOME/.bashrc
		fi
		;;
	esac
fi

if [ -n "$profile" ]; then
	if grep -qF "$line" "$profile" 2>/dev/null; then
		path_note="PATH already includes $dir in $profile"
	elif [ -d "${profile%/*}" ] && (printf '%s\n' "$line" >>"$profile") 2>/dev/null; then
		path_note="Added $dir to PATH in $profile"
	else
		path_note="Add $dir to PATH by hand, one line in the profile you use: $line"
	fi
else
	path_note="Add $dir to PATH by hand, one line in the profile you use: $line"
fi

printf 'Installed %s\n' "$dir/wok"
printf 'Installed %s\n' "$dir/wok-daemon"
printf '%s\n' "$path_note"
printf 'Next: install the Kitty integration files from kitty/; see the "Install" section of README.md.\n'
printf 'Only the two binaries were installed: no runtime or package manager, and no Rust toolchain.\n'
