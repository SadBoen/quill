#!/usr/bin/env bash
# 取回只读参考源码：vendor/goose 与 .octop-ref/octop。
#
# 为什么它们不在仓库里：vendor/goose 有 238MB，其中大头是 documentation 的
# 博客图片与 meetup 照片 —— 对编译一行都用不上，却让 clone 变慢、仓库膨胀。
# 记 UPSTREAM.md 里已经写了出处与 pin，需要时按 pin 精确拉回来即可。
#
# 用法：bash .scripts/fetch-vendor.sh
set -euo pipefail
cd "$(dirname "$0")/.."

# 与 UPSTREAM.md 保持一致。改这里之前先改那份文件。
GOOSE_REPO="https://github.com/block/goose.git"
GOOSE_PIN="v1.53.0"
OCTOP_REPO="${OCTOP_REPO:-?}"
OCTOP_PIN="eb280112"

say() { printf '\n== %s ==\n' "$1"; }

say "goose（vendor/goose）"
if [ -d vendor/goose/.git ]; then
  echo "  已存在，跳过。要更新就手动：git -C vendor/goose checkout $GOOSE_PIN"
else
  mkdir -p vendor
  git clone --filter=blob:none --no-checkout "$GOOSE_REPO" vendor/goose
  git -C vendor/goose checkout -q "$GOOSE_PIN"
  echo "  已取到 $GOOSE_PIN"
fi

say "octop（.octop-ref/octop）"
if [ -d .octop-ref/octop/.git ]; then
  echo "  已存在，跳过。"
else
  if [ "$OCTOP_REPO" = "?" ]; then
    cat >&2 <<'EOF'
  没有填 OCTOP_REPO。

  Octop 是设计参考，取它的地址后重跑：
    OCTOP_REPO=<octop 的 git 地址> bash .scripts/fetch-vendor.sh

  出处与 pin 见 UPSTREAM.md。
EOF
    exit 1
  fi
  mkdir -p .octop-ref
  git clone --filter=blob:none --no-checkout "$OCTOP_REPO" .octop-ref/octop
  git -C .octop-ref/octop checkout -q "$OCTOP_PIN"
  echo "  已取到 $OCTOP_PIN"
fi

say "校验"
[ -f vendor/goose/Cargo.toml ] && echo "  ✓ vendor/goose" || echo "  ✗ vendor/goose 不可用"
if [ -d .octop-ref/octop/src/octop/infra/agents/experts/library ]; then
  echo "  ✓ .octop-ref 专家库源数据（.library-check.mjs 可以跑了）"
else
  echo "  ✗ .octop-ref 专家库源数据缺失，.library-check.mjs 会拒绝运行"
fi

cat <<'EOF'

下一步：
  node .library-check.mjs   # 专家库源 vs 产物双向比对
  cargo test --workspace    # 编译期不依赖 vendor（goose 不会被引用）
EOF
