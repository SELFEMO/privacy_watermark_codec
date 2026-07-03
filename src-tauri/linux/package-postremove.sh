#!/bin/sh
set -eu

# 卸载后刷新桌面入口数据库，避免 GNOME 菜单和 Dock 继续引用已经移除的系统级入口。
# Refreshing the desktop-entry database after removal prevents GNOME menus and Dock from keeping references to the removed system entry.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

# 卸载阶段同样刷新 hicolor 主题缓存，避免图标文件已删除但缓存仍指向旧资源。
# The hicolor theme cache is refreshed during removal as well so removed icon files are not kept through stale cache references.
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
