//! Seed builders for the graphical suites.
//!
//! The shape mirrors `library::LibraryEntry` exactly. It is written by hand
//! rather than generated so that a field added on the Rust side without a
//! `#[serde(default)]` surfaces here as a failing import instead of a silently
//! empty library — the LOT-13 lesson, applied to the fixtures this suite runs
//! on.

export interface IndexEntry {
  app_id: string;
  name: string;
  icon?: string | null;
  file_name: string;
  added_at: string;
  updated_at: string;
  has_fix?: boolean;
  hidden?: boolean;
  tags?: string[];
}

export function entry(app_id: string, name: string, extra: Partial<IndexEntry> = {}): IndexEntry {
  return {
    app_id,
    name,
    icon: null,
    file_name: `${app_id}.lua`,
    // Fixed timestamps: a test that sorts by date must not depend on the clock.
    added_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-02T00:00:00Z',
    has_fix: false,
    hidden: false,
    tags: [],
    ...extra,
  };
}

/// A small library with names and AppIDs chosen so each filter test has exactly
/// one unambiguous match.
export const SAMPLE = [
  entry('264710', 'Subnautica', { tags: ['survie'] }),
  entry('1091500', 'Cyberpunk 2077', { tags: ['rpg'] }),
  entry('292030', 'The Witcher 3', { tags: ['rpg'] }),
];

export const SAMPLE_LUA: Record<string, string> = {
  '264710': 'addappid(264710)\n',
  '1091500': 'addappid(1091500)\n',
  '292030': 'addappid(292030)\n',
};

/// A library big enough to cross `VIRTUAL_THRESHOLD` (100) in LibraryView.
/// Names are zero-padded so a filter on "Jeu 149" matches exactly one, and that
/// one sits far past the first rendered window — which is the point.
export const MANY = Array.from({ length: 150 }, (_, i) =>
  entry(String(500000 + i), `Jeu ${String(i + 1).padStart(3, '0')}`),
);

export const MANY_LUA: Record<string, string> = Object.fromEntries(
  MANY.map((e) => [e.app_id, `addappid(${e.app_id})
`]),
);
