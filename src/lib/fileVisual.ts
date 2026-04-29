import {
  File as FileIcon,
  FileArchive,
  FileAudio2,
  FileCode,
  FileCog,
  FileImage,
  FileJson,
  FileSpreadsheet,
  FileTerminal,
  FileText,
  FileVideo,
  Package,
} from "lucide-solid";

const PACKAGE_FILE_NAMES = new Set([
  "package.json",
  "package-lock.json",
  "pnpm-lock.yaml",
  "yarn.lock",
  "bun.lock",
  "bun.lockb",
  "cargo.toml",
  "cargo.lock",
  "go.mod",
  "go.sum",
  "composer.json",
  "composer.lock",
  "gemfile",
  "gemfile.lock",
  "pyproject.toml",
  "poetry.lock",
  "requirements.txt",
]);

const CONFIG_FILE_NAMES = new Set([
  ".gitignore",
  ".gitattributes",
  ".npmrc",
  ".nvmrc",
  ".editorconfig",
  ".prettierrc",
  ".prettierignore",
  ".eslintrc",
  ".eslintignore",
  "tauri.conf.json",
  "tsconfig.json",
  "tsconfig.node.json",
  "info.plist",
  "rust-toolchain.toml",
]);

const DOC_EXTENSIONS = new Set(["md", "mdx", "txt", "rtf", "adoc"]);
const CODE_EXTENSIONS = new Set([
  "c",
  "cc",
  "cpp",
  "cs",
  "css",
  "cxx",
  "go",
  "h",
  "hpp",
  "html",
  "java",
  "js",
  "jsx",
  "kt",
  "less",
  "lua",
  "mjs",
  "mts",
  "php",
  "py",
  "rb",
  "rs",
  "sass",
  "scala",
  "scss",
  "sql",
  "svelte",
  "swift",
  "ts",
  "tsx",
  "vue",
  "xml",
]);
const CONFIG_EXTENSIONS = new Set(["conf", "config", "ini", "json", "jsonc", "plist", "toml", "yaml", "yml"]);
const SHELL_EXTENSIONS = new Set(["bash", "fish", "ps1", "sh", "zsh"]);
const IMAGE_EXTENSIONS = new Set(["avif", "bmp", "gif", "ico", "jpeg", "jpg", "png", "svg", "tif", "tiff", "webp"]);
const AUDIO_EXTENSIONS = new Set(["aac", "flac", "m4a", "mp3", "ogg", "wav"]);
const VIDEO_EXTENSIONS = new Set(["avi", "m4v", "mkv", "mov", "mp4", "webm"]);
const ARCHIVE_EXTENSIONS = new Set(["7z", "bz2", "gz", "rar", "tar", "tgz", "xz", "zip"]);
const SPREADSHEET_EXTENSIONS = new Set(["csv", "ods", "tsv", "xls", "xlsx"]);

export type FileVisual = {
  Icon: typeof FileIcon;
  toneClass: string;
};

function fileExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "";
  return name.slice(dot + 1).toLowerCase();
}

export function getFileVisual(name: string): FileVisual {
  const normalized = name.toLowerCase();
  const ext = fileExtension(normalized);

  if (PACKAGE_FILE_NAMES.has(normalized)) {
    return { Icon: Package, toneClass: "file-glyph-package" };
  }
  if (
    CONFIG_FILE_NAMES.has(normalized) ||
    normalized.startsWith(".env") ||
    normalized.endsWith(".config.ts") ||
    normalized.endsWith(".config.js") ||
    normalized.endsWith(".config.mjs") ||
    normalized.endsWith(".config.mts")
  ) {
    return { Icon: FileCog, toneClass: "file-glyph-config" };
  }
  if (SPREADSHEET_EXTENSIONS.has(ext)) {
    return { Icon: FileSpreadsheet, toneClass: "file-glyph-sheet" };
  }
  if (IMAGE_EXTENSIONS.has(ext)) {
    return { Icon: FileImage, toneClass: "file-glyph-media" };
  }
  if (AUDIO_EXTENSIONS.has(ext)) {
    return { Icon: FileAudio2, toneClass: "file-glyph-media" };
  }
  if (VIDEO_EXTENSIONS.has(ext)) {
    return { Icon: FileVideo, toneClass: "file-glyph-media" };
  }
  if (ARCHIVE_EXTENSIONS.has(ext)) {
    return { Icon: FileArchive, toneClass: "file-glyph-archive" };
  }
  if (SHELL_EXTENSIONS.has(ext) || normalized === "dockerfile" || normalized === "makefile") {
    return { Icon: FileTerminal, toneClass: "file-glyph-shell" };
  }
  if (DOC_EXTENSIONS.has(ext) || normalized === "license" || normalized === "copying" || normalized === "notice") {
    return { Icon: FileText, toneClass: "file-glyph-doc" };
  }
  if (CONFIG_EXTENSIONS.has(ext)) {
    return { Icon: FileJson, toneClass: "file-glyph-config" };
  }
  if (CODE_EXTENSIONS.has(ext)) {
    return { Icon: FileCode, toneClass: "file-glyph-code" };
  }
  return { Icon: FileIcon, toneClass: "file-glyph-default" };
}
