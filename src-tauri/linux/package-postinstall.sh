#!/bin/sh
set -eu

# 安装包写入桌面入口和 hicolor 图标后需要刷新缓存，否则 GNOME Dock 可能继续沿用旧的未知软件图标。
# After the package writes the desktop entry and hicolor icons, caches must be refreshed so GNOME Dock does not keep the old unknown-app icon.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

# 图标主题缓存失败不应阻断安装；失败时桌面环境仍可在下一次缓存刷新或重新登录后发现图标。
# Icon-theme cache refresh failures must not block installation; the desktop can still find the icon after the next cache refresh or login.
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
