<script lang="ts" module>
  /** Accent + soft-fill pair per icon, so each glyph carries its own meaning. */
  const PALETTE: Record<string, [string, string]> = {
    // Navigation
    search: ["var(--color-sky)", "var(--color-sky-soft)"],
    library: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    news: ["var(--color-sky)", "var(--color-sky-soft)"],
    tools: ["var(--color-peach)", "var(--color-peach-soft)"],
    logs: ["var(--color-azure-600)", "var(--color-azure-100)"],
    settings: ["var(--color-azure-700)", "var(--color-azure-100)"],

    // Status
    check: ["var(--color-mint)", "var(--color-mint-soft)"],
    alert: ["var(--color-peach)", "var(--color-peach-soft)"],
    error: ["var(--color-rose)", "var(--color-rose-soft)"],
    info: ["var(--color-sky)", "var(--color-sky-soft)"],
    shield: ["var(--color-mint)", "var(--color-mint-soft)"],
    clock: ["var(--color-peach)", "var(--color-peach-soft)"],

    // Actions
    download: ["var(--color-sky)", "var(--color-sky-soft)"],
    upload: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    folder: ["var(--color-peach)", "var(--color-peach-soft)"],
    trash: ["var(--color-rose)", "var(--color-rose-soft)"],
    refresh: ["var(--color-sky)", "var(--color-sky-soft)"],
    copy: ["var(--color-azure-600)", "var(--color-azure-100)"],
    edit: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    play: ["var(--color-mint)", "var(--color-mint-soft)"],
    wrench: ["var(--color-peach)", "var(--color-peach-soft)"],
    broom: ["var(--color-rose)", "var(--color-rose-soft)"],
    x: ["var(--color-azure-700)", "var(--color-azure-100)"],
    "chevron-left": ["var(--color-azure-700)", "var(--color-azure-100)"],
    "chevron-right": ["var(--color-azure-700)", "var(--color-azure-100)"],
    plus: ["var(--color-mint)", "var(--color-mint-soft)"],
    eye: ["var(--color-sky)", "var(--color-sky-soft)"],
    "eye-off": ["var(--color-rose)", "var(--color-rose-soft)"],
    more: ["var(--color-azure-700)", "var(--color-azure-100)"],

    // Domain
    steam: ["var(--color-azure-700)", "var(--color-azure-200)"],
    gamepad: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    globe: ["var(--color-sky)", "var(--color-sky-soft)"],
    offline: ["var(--color-sky)", "var(--color-sky-soft)"],
    patch: ["var(--color-mint)", "var(--color-mint-soft)"],
    archive: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    save: ["var(--color-azure-600)", "var(--color-azure-100)"],
    sparkle: ["var(--color-peach)", "var(--color-peach-soft)"],
    discord: ["#5865f2", "#e0e3ff"],
    palette: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    sun: ["var(--color-peach)", "var(--color-peach-soft)"],
    moon: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    layers: ["var(--color-azure-600)", "var(--color-azure-100)"],
    image: ["var(--color-sky)", "var(--color-sky-soft)"],
    lock: ["var(--color-mint)", "var(--color-mint-soft)"],
    "arrow-up-down": ["var(--color-azure-700)", "var(--color-azure-100)"],
    tag: ["var(--color-lilac)", "var(--color-lilac-soft)"],
    update: ["var(--color-sky)", "var(--color-sky-soft)"],
    select: ["var(--color-azure-600)", "var(--color-azure-100)"],
    checkbox: ["var(--color-azure-600)", "var(--color-azure-100)"],
  };
</script>

<script lang="ts">
  let {
    name,
    size = 18,
    class: cls = "",
    /** `duo` uses the icon's own colours, `current` inherits the text colour. */
    tone = "duo",
  }: {
    name: string;
    size?: number;
    class?: string;
    tone?: "duo" | "current";
  } = $props();

  const pair = $derived(PALETTE[name] ?? ["var(--color-azure-700)", "var(--color-azure-100)"]);
  const style = $derived(
    tone === "current"
      ? "--ic: currentColor; --ic-soft: currentColor; --ic-soft-opacity: 0.22;"
      : `--ic: ${pair[0]}; --ic-soft: ${pair[1]}; --ic-soft-opacity: 1;`,
  );
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="var(--ic)"
  stroke-width="1.9"
  stroke-linecap="round"
  stroke-linejoin="round"
  class={cls}
  {style}
  aria-hidden="true"
>
  <!-- Soft shapes sit behind the outline to give each icon its colour. -->
  <g fill="var(--ic-soft)" stroke="none" opacity="var(--ic-soft-opacity)">
    {#if name === "search"}
      <circle cx="11" cy="11" r="7" />
    {:else if name === "library"}
      <rect x="3" y="4" width="4" height="16" rx="1.2" />
      <rect x="9" y="7" width="4" height="13" rx="1.2" />
    {:else if name === "news"}
      <path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Z" />
    {:else if name === "tools"}
      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
    {:else if name === "logs"}
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
    {:else if name === "settings"}
      <circle cx="12" cy="12" r="8.4" />
    {:else if name === "check"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "alert" || name === "error"}
      <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
    {:else if name === "info"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "shield" || name === "lock"}
      <path d="M12 2.5 20 6v6c0 5-3.4 8.6-8 9.5C7.4 20.6 4 17 4 12V6z" />
    {:else if name === "clock"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "download" || name === "upload"}
      <path d="M3 14v5a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-5z" />
    {:else if name === "folder"}
      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
    {:else if name === "trash"}
      <path d="M6 7h12v13a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2z" />
    {:else if name === "refresh"}
      <circle cx="12" cy="12" r="8.6" />
    {:else if name === "copy"}
      <rect x="9" y="9" width="13" height="13" rx="2" />
    {:else if name === "edit"}
      <path d="M4 20l4-1 9.5-9.5-3-3L5 16z" />
    {:else if name === "play"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "wrench"}
      <path d="M15.5 3a6 6 0 0 0-5.3 8.7L3 19l2 2 7.3-7.2A6 6 0 1 0 15.5 3z" />
    {:else if name === "broom"}
      <path d="M4 21h16l-2-7H6z" />
    {:else if name === "x"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "plus"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "eye" || name === "eye-off"}
      <path d="M12 5c5 0 9 4.2 10 7-1 2.8-5 7-10 7S3 14.8 2 12c1-2.8 5-7 10-7z" />
    {:else if name === "steam"}
      <circle cx="12" cy="12" r="9.2" />
    {:else if name === "gamepad"}
      <rect x="2" y="6" width="20" height="12" rx="4" />
    {:else if name === "globe"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "offline"}
      <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" />
    {:else if name === "patch"}
      <path d="M10 3h4v3.2a1.6 1.6 0 0 0 2.7 1.1l2.2 2.2A1.6 1.6 0 0 0 20 12v4h-3.2a1.6 1.6 0 0 0-1.1 2.7L14 21h-4v-3.2A1.6 1.6 0 0 0 7.3 16.7L5 14.5V10h3.2A1.6 1.6 0 0 0 9.3 7.3z" />
    {:else if name === "archive"}
      <path d="M3 8h18v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    {:else if name === "save"}
      <path d="M4 3h13l4 4v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" />
    {:else if name === "sparkle"}
      <path d="M12 2.5 14 9l6.5 2-6.5 2-2 6.5-2-6.5L3.5 11 10 9z" />
    {:else if name === "discord"}
      <path d="M19.3 5.6A16.6 16.6 0 0 0 15.1 4.3l-.2.4a15.4 15.4 0 0 0-5.8 0l-.2-.4A16.6 16.6 0 0 0 4.7 5.6C2.1 9.5 1.4 13.3 1.7 17a16.7 16.7 0 0 0 5.1 2.6l1-1.7a10.9 10.9 0 0 1-1.7-.8l.4-.3a11.9 11.9 0 0 0 10 0l.4.3a10.9 10.9 0 0 1-1.7.8l1 1.7a16.7 16.7 0 0 0 5.1-2.6c.4-4.3-.6-8.1-2-11.4z" />
    {:else if name === "palette"}
      <path d="M12 3a9 9 0 0 0 0 18c1.4 0 2-.9 2-1.8 0-1.4-1.2-1.7-1.2-2.7 0-.8.7-1.5 1.6-1.5H16a5 5 0 0 0 5-5c0-3.9-4-7-9-7z" />
    {:else if name === "sun"}
      <circle cx="12" cy="12" r="4.6" />
    {:else if name === "moon"}
      <path d="M20 14.5A8.5 8.5 0 0 1 9.5 4 8.5 8.5 0 1 0 20 14.5z" />
    {:else if name === "layers"}
      <path d="m12 2.6 9 5-9 5-9-5z" />
    {:else if name === "image"}
      <rect x="3" y="4" width="18" height="16" rx="2" />
    {:else if name === "arrow-up-down"}
      <path d="M12 3v18M8 15l-4-4 4-4M16 9l4 4-4 4" />
    {:else if name === "tag"}
      <path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" />
      <line x1="7" y1="7" x2="7.01" y2="7" />
    {:else if name === "update"}
      <circle cx="12" cy="12" r="9" />
    {:else if name === "select"}
      <rect x="3" y="3" width="18" height="18" rx="4" />
    {:else if name === "checkbox"}
      <rect x="3" y="3" width="18" height="18" rx="4" />
    {:else if name === "chevron-left" || name === "chevron-right"}
      <circle cx="12" cy="12" r="9" />
    {/if}
  </g>

  {#if name === "search"}
    <circle cx="11" cy="11" r="7" />
    <path d="m21 21-4.3-4.3" />
  {:else if name === "library"}
    <rect x="3" y="4" width="4" height="16" rx="1.2" />
    <rect x="9" y="7" width="4" height="13" rx="1.2" />
    <path d="m16 6.5 3.6 12.1" />
  {:else if name === "news"}
    <path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2" />
    <path d="M18 14h-8" />
    <path d="M15 18h-5" />
    <path d="M10 6h8v4h-8V6Z" />
  {:else if name === "tools"}
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
  {:else if name === "logs"}
    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
    <polyline points="14 2 14 8 20 8" />
    <line x1="16" y1="13" x2="8" y2="13" />
    <line x1="16" y1="17" x2="8" y2="17" />
  {:else if name === "settings"}
    <circle cx="12" cy="12" r="3.1" />
    <path d="M12 2.6v2.1M12 19.3v2.1M4.4 4.4l1.5 1.5M18.1 18.1l1.5 1.5M2.6 12h2.1M19.3 12h2.1M4.4 19.6l1.5-1.5M18.1 5.9l1.5-1.5" />
  {:else if name === "check"}
    <circle cx="12" cy="12" r="9" />
    <path d="m8 12.4 2.7 2.7L16 9.6" />
  {:else if name === "alert" || name === "error"}
    <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
    <line x1="12" y1="9" x2="12" y2="13.5" />
    <line x1="12" y1="17.2" x2="12.01" y2="17.2" />
  {:else if name === "info"}
    <circle cx="12" cy="12" r="9" />
    <line x1="12" y1="16.5" x2="12" y2="11.5" />
    <line x1="12" y1="8" x2="12.01" y2="8" />
  {:else if name === "shield"}
    <path d="M12 2.5 20 6v6c0 5-3.4 8.6-8 9.5C7.4 20.6 4 17 4 12V6z" />
    <path d="m9 12 2.2 2.2L15.5 10" />
  {:else if name === "lock"}
    <path d="M12 2.5 20 6v6c0 5-3.4 8.6-8 9.5C7.4 20.6 4 17 4 12V6z" />
    <circle cx="12" cy="11.5" r="1.6" />
    <path d="M12 13.1v2.6" />
  {:else if name === "clock"}
    <circle cx="12" cy="12" r="9" />
    <polyline points="12 7 12 12 15.2 13.8" />
  {:else if name === "download"}
    <path d="M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-5" />
    <polyline points="7.5 10 12 14.5 16.5 10" />
    <line x1="12" y1="14.5" x2="12" y2="3" />
  {:else if name === "upload"}
    <path d="M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-5" />
    <polyline points="7.5 7.5 12 3 16.5 7.5" />
    <line x1="12" y1="3" x2="12" y2="14.5" />
  {:else if name === "folder"}
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
  {:else if name === "trash"}
    <polyline points="3.5 6 6 6 20.5 6" />
    <path d="M18 6v14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2" />
    <line x1="10" y1="11" x2="10" y2="17" />
    <line x1="14" y1="11" x2="14" y2="17" />
  {:else if name === "refresh"}
    <polyline points="21.5 4 21.5 9.6 15.9 9.6" />
    <path d="M19.5 15a8 8 0 1 1-1.9-8.3l3.9 2.9" />
  {:else if name === "copy"}
    <rect x="9" y="9" width="13" height="13" rx="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  {:else if name === "edit"}
    <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
    <path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z" />
  {:else if name === "play"}
    <circle cx="12" cy="12" r="9" />
    <path d="M10.2 8.6 15.6 12l-5.4 3.4z" fill="var(--ic)" />
  {:else if name === "wrench"}
    <path d="M15.5 3a6 6 0 0 0-5.3 8.7L3 19l2 2 7.3-7.2A6 6 0 1 0 15.5 3z" />
    <circle cx="16.4" cy="7.6" r="1.5" />
  {:else if name === "broom"}
    <path d="M15 3 8.5 9.5" />
    <path d="M6 14 4 21h16l-2-7z" />
    <path d="M6 14h12" />
    <path d="M11 14v7M16 14v7" />
  {:else if name === "x"}
    <line x1="17" y1="7" x2="7" y2="17" />
    <line x1="7" y1="7" x2="17" y2="17" />
  {:else if name === "plus"}
    <line x1="12" y1="7" x2="12" y2="17" />
    <line x1="7" y1="12" x2="17" y2="12" />
  {:else if name === "eye"}
    <path d="M12 5c5 0 9 4.2 10 7-1 2.8-5 7-10 7S3 14.8 2 12c1-2.8 5-7 10-7z" />
    <circle cx="12" cy="12" r="3" />
  {:else if name === "eye-off"}
    <path d="M12 5c5 0 9 4.2 10 7-.3.9-1 2-2 3.1M6.6 6.6C4.3 8 2.7 10.3 2 12c1 2.8 5 7 10 7 1.6 0 3-.4 4.3-1.1" />
    <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
    <line x1="3" y1="3" x2="21" y2="21" />
  {:else if name === "more"}
    <circle cx="12" cy="5" r="1.4" fill="var(--ic)" />
    <circle cx="12" cy="12" r="1.4" fill="var(--ic)" />
    <circle cx="12" cy="19" r="1.4" fill="var(--ic)" />
  {:else if name === "steam"}
    <circle cx="12" cy="12" r="9.2" />
    <circle cx="15.2" cy="9" r="2.6" />
    <circle cx="8.2" cy="14.6" r="2.2" />
    <path d="m13.3 10.8-3.4 2.4" />
    <path d="M2.9 15.2 6.3 13.6" />
  {:else if name === "gamepad"}
    <rect x="2" y="6" width="20" height="12" rx="4" />
    <line x1="6.5" y1="12" x2="10" y2="12" />
    <line x1="8.2" y1="10.2" x2="8.2" y2="13.8" />
    <line x1="15" y1="13.4" x2="15.01" y2="13.4" />
    <line x1="18" y1="10.8" x2="18.01" y2="10.8" />
  {:else if name === "globe"}
    <circle cx="12" cy="12" r="9" />
    <line x1="3" y1="12" x2="21" y2="12" />
    <path d="M12 3a14 14 0 0 1 3.6 9A14 14 0 0 1 12 21a14 14 0 0 1-3.6-9A14 14 0 0 1 12 3z" />
  {:else if name === "offline"}
    <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" />
    <line x1="3.5" y1="3.5" x2="20.5" y2="20.5" />
  {:else if name === "patch"}
    <path d="M10 3h4v3.2a1.6 1.6 0 0 0 2.7 1.1l2.2 2.2A1.6 1.6 0 0 0 20 12v4h-3.2a1.6 1.6 0 0 0-1.1 2.7L14 21h-4v-3.2A1.6 1.6 0 0 0 7.3 16.7L5 14.5V10h3.2A1.6 1.6 0 0 0 9.3 7.3z" />
  {:else if name === "archive"}
    <rect x="2.5" y="3.5" width="19" height="5" rx="1.4" />
    <path d="M4 8.5V19a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8.5" />
    <line x1="10" y1="12.5" x2="14" y2="12.5" />
  {:else if name === "save"}
    <path d="M4 3h13l4 4v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" />
    <path d="M7.5 3v6h8V3" />
    <rect x="7.5" y="13" width="9" height="6" rx="1" />
  {:else if name === "sparkle"}
    <path d="M12 2.5 14 9l6.5 2-6.5 2-2 6.5-2-6.5L3.5 11 10 9z" />
    <path d="M18.5 3v3M20 4.5h-3" />
  {:else if name === "discord"}
    <path d="M19.3 5.6A16.6 16.6 0 0 0 15.1 4.3l-.2.4a15.4 15.4 0 0 0-5.8 0l-.2-.4A16.6 16.6 0 0 0 4.7 5.6C2.1 9.5 1.4 13.3 1.7 17a16.7 16.7 0 0 0 5.1 2.6l1-1.7a10.9 10.9 0 0 1-1.7-.8l.4-.3a11.9 11.9 0 0 0 10 0l.4.3a10.9 10.9 0 0 1-1.7.8l1 1.7a16.7 16.7 0 0 0 5.1-2.6c.4-4.3-.6-8.1-2-11.4z" />
    <ellipse cx="9" cy="12.6" rx="1.5" ry="1.8" fill="var(--ic)" stroke="none" />
    <ellipse cx="15" cy="12.6" rx="1.5" ry="1.8" fill="var(--ic)" stroke="none" />
  {:else if name === "palette"}
    <path d="M12 3a9 9 0 0 0 0 18c1.4 0 2-.9 2-1.8 0-1.4-1.2-1.7-1.2-2.7 0-.8.7-1.5 1.6-1.5H16a5 5 0 0 0 5-5c0-3.9-4-7-9-7z" />
    <circle cx="8" cy="9.5" r="1.1" fill="var(--ic)" stroke="none" />
    <circle cx="12" cy="7.5" r="1.1" fill="var(--ic)" stroke="none" />
    <circle cx="16" cy="9.8" r="1.1" fill="var(--ic)" stroke="none" />
    <circle cx="7.2" cy="14" r="1.1" fill="var(--ic)" stroke="none" />
  {:else if name === "sun"}
    <circle cx="12" cy="12" r="4.6" />
    <path d="M12 2.4v2.2M12 19.4v2.2M4.2 4.2l1.6 1.6M18.2 18.2l1.6 1.6M2.4 12h2.2M19.4 12h2.2M4.2 19.8l1.6-1.6M18.2 5.8l1.6-1.6" />
  {:else if name === "moon"}
    <path d="M20 14.5A8.5 8.5 0 0 1 9.5 4 8.5 8.5 0 1 0 20 14.5z" />
  {:else if name === "layers"}
    <path d="m12 2.6 9 5-9 5-9-5z" />
    <path d="m3 12.4 9 5 9-5" />
    <path d="m3 16.8 9 5 9-5" />
  {:else if name === "image"}
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <circle cx="9" cy="9.5" r="1.7" />
    <path d="m5 18 4.5-4.5 3 3L16 13l3.5 3.5" />
  {:else if name === "arrow-up-down"}
    <path d="M12 3v18M8 15l-4-4 4-4M16 9l4 4-4 4" />
  {:else if name === "tag"}
    <path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" />
    <line x1="7" y1="7" x2="7.01" y2="7" />
  {:else if name === "update"}
    <circle cx="12" cy="12" r="9" />
    <polyline points="8 12 12 8 16 12" />
    <line x1="12" y1="8" x2="12" y2="16" />
  {:else if name === "select"}
    <rect x="3.5" y="3.5" width="17" height="17" rx="3.5" />
    <path d="m8.3 12.3 2.7 2.7 5-5.6" />
  {:else if name === "checkbox"}
    <path d="m5.5 12.5 4.2 4.2L18.5 7.5" />
  {:else if name === "chevron-left"}
    <path d="M14 7.5 9.5 12l4.5 4.5" />
  {:else if name === "chevron-right"}
    <path d="M10 7.5 14.5 12 10 16.5" />
  {/if}
</svg>
