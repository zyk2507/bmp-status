#!/usr/bin/env bash
# Shared environment for containerized builds. This file is meant to be sourced.
#
# - Detects whether the build host is in mainland China.
# - In China, requires an HTTP(S) proxy and caches it in .build.env.
# - Selects the CERNET (China Education and Research Network) mirror for
#   Alpine apk and Debian apt based container builds.
#
# CERNET mirror documentation: https://help.mirrors.cernet.edu.cn/debian/

set -euo pipefail

BUILD_ENV_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_ENV_FILE="${BUILD_ENV_FILE:-$BUILD_ENV_ROOT/.build.env}"

# Values explicitly supplied in the environment must win over cached values.
ENV_PROXY_OVERRIDE="${PROXY:-${HTTPS_PROXY:-${https_proxy:-${HTTP_PROXY:-${http_proxy:-}}}}}"
ENV_ALPINE_MIRROR_OVERRIDE="${ALPINE_MIRROR:-}"
ENV_DEBIAN_MIRROR_OVERRIDE="${DEBIAN_MIRROR:-}"
ENV_DEBIAN_SECURITY_MIRROR_OVERRIDE="${DEBIAN_SECURITY_MIRROR:-}"

# Load cached values. The file is gitignored and written with mode 0600.
if [[ -f "$BUILD_ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$BUILD_ENV_FILE"
    set +a
fi

[[ -n "$ENV_PROXY_OVERRIDE" ]] && PROXY="$ENV_PROXY_OVERRIDE"
[[ -n "$ENV_ALPINE_MIRROR_OVERRIDE" ]] && ALPINE_MIRROR="$ENV_ALPINE_MIRROR_OVERRIDE"
[[ -n "$ENV_DEBIAN_MIRROR_OVERRIDE" ]] && DEBIAN_MIRROR="$ENV_DEBIAN_MIRROR_OVERRIDE"
[[ -n "$ENV_DEBIAN_SECURITY_MIRROR_OVERRIDE" ]] && DEBIAN_SECURITY_MIRROR="$ENV_DEBIAN_SECURITY_MIRROR_OVERRIDE"

MIRROR_HELP_URL="https://help.mirrors.cernet.edu.cn/debian/"
ALPINE_MIRROR="${ALPINE_MIRROR:-mirrors.cernet.edu.cn}"
DEBIAN_MIRROR="${DEBIAN_MIRROR:-mirrors.cernet.edu.cn}"
DEBIAN_SECURITY_MIRROR="${DEBIAN_SECURITY_MIRROR:-mirrors.cernet.edu.cn}"

# Accept PROXY, HTTPS_PROXY, or HTTP_PROXY in that order.
PROXY="${PROXY:-${HTTPS_PROXY:-${https_proxy:-${HTTP_PROXY:-${http_proxy:-}}}}}"
if [[ -n "$PROXY" && "$PROXY" != *"://"* ]]; then
    PROXY="http://$PROXY"
fi

timezone_in_china() {
    local tz=""
    if [[ -n "${TZ:-}" ]]; then
        tz="$TZ"
    elif [[ -f /etc/timezone ]]; then
        tz="$(tr -d '[:space:]' < /etc/timezone)"
    elif command -v timedatectl >/dev/null 2>&1; then
        tz="$(timedatectl show -p Timezone --value 2>/dev/null || true)"
    fi
    case "$tz" in
        Asia/Shanghai|Asia/Chongqing|Asia/Urumqi|Asia/Harbin|PRC)
            return 0
            ;;
    esac
    return 1
}

country_from_ip_service() {
    local url="" country=""
    command -v curl >/dev/null 2>&1 || return 1
    for url in \
        "https://ipinfo.io/country" \
        "https://ifconfig.co/country-iso" \
        "https://api.country.is"; do
        # Direct request only: using a proxy here could hide the real location.
        country="$(curl --noproxy '*' -4 -fsS --max-time 4 "$url" 2>/dev/null \
            | tr -d '[:space:]' | tr '[:lower:]' '[:upper:]' || true)"
        if [[ "$country" == "CN" ]]; then
            return 0
        fi
        if [[ "$country" =~ ^[A-Z]{2}$ ]]; then
            return 1
        fi
    done
    return 1
}

detect_china() {
    case "${FORCE_CN:-}" in
        1|true|TRUE|yes|YES)
            return 0
            ;;
        0|false|FALSE|no|NO)
            return 1
            ;;
    esac
    if timezone_in_china; then
        return 0
    fi
    if country_from_ip_service; then
        return 0
    fi
    return 1
}

IN_CHINA=0
if detect_china; then
    IN_CHINA=1
fi

if [[ "$IN_CHINA" == "1" && -z "$PROXY" ]]; then
    if [[ "${ALLOW_NO_PROXY:-0}" == "1" ]]; then
        echo "==> China network detected, but ALLOW_NO_PROXY=1 is set; continuing without a proxy." >&2
    elif [[ -t 0 ]]; then
        echo "==> China network detected. Container image pulls and package downloads may be slow." >&2
        echo "    Enter an HTTP(S) proxy URL (for example http://192.168.1.114:7890)." >&2
        printf '    Proxy: ' >&2
        read -r PROXY || true
        if [[ -z "$PROXY" ]]; then
            echo "ERROR: a proxy is required for containerized builds in China." >&2
            echo "       Set PROXY=http://host:port, or use ALLOW_NO_PROXY=1 to override." >&2
            return 1 2>/dev/null || exit 1
        fi
        if [[ "$PROXY" != *"://"* ]]; then
            PROXY="http://$PROXY"
        fi
    else
        echo "ERROR: China network detected and no proxy was provided." >&2
        echo "       Run with PROXY=http://host:port make <target>." >&2
        echo "       Use ALLOW_NO_PROXY=1 only if direct access works." >&2
        return 1 2>/dev/null || exit 1
    fi
fi

# Export both upper and lower case variants because different tools use both.
if [[ -n "$PROXY" ]]; then
    export PROXY
    export HTTP_PROXY="$PROXY"
    export HTTPS_PROXY="$PROXY"
    export http_proxy="$PROXY"
    export https_proxy="$PROXY"
fi
export NO_PROXY="${NO_PROXY:-localhost,127.0.0.1,::1}"
export no_proxy="$NO_PROXY"

export ALPINE_MIRROR
export DEBIAN_MIRROR
export DEBIAN_SECURITY_MIRROR
export MIRROR_HELP_URL

# Cache the proxy and mirrors so repeated make targets do not prompt again.
if [[ "$IN_CHINA" == "1" && -n "$PROXY" ]]; then
    BUILD_ENV_UMASK="$(umask)"
    umask 077
    {
        printf 'PROXY=%q\n' "$PROXY"
        printf 'ALPINE_MIRROR=%q\n' "$ALPINE_MIRROR"
        printf 'DEBIAN_MIRROR=%q\n' "$DEBIAN_MIRROR"
        printf 'DEBIAN_SECURITY_MIRROR=%q\n' "$DEBIAN_SECURITY_MIRROR"
    } > "$BUILD_ENV_FILE"
    umask "$BUILD_ENV_UMASK"
fi

if [[ "$IN_CHINA" == "1" ]]; then
    echo "==> Network: mainland China detected."
else
    echo "==> Network: mainland China not detected (use FORCE_CN=1 to override)."
fi
echo "==> Alpine mirror: https://${ALPINE_MIRROR}"
echo "==> Debian mirror: https://${DEBIAN_MIRROR} (help: ${MIRROR_HELP_URL})"
if [[ -n "$PROXY" ]]; then
    echo "==> Proxy: ${PROXY}"
fi
