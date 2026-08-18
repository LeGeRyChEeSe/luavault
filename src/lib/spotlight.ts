export function spotlightTitle(
  gameName: string,
  steamName: string | null | undefined,
): string {
  return steamName ?? gameName;
}

export function displayCorrection(
  current: { name: string; icon?: string | null },
  details: { name: string; header_image?: string | null } | null,
): { name: string; icon: string | null } | null {
  if (!details?.name) return null;
  const nameChanged = details.name !== current.name;
  const iconChanged = !!details.header_image && details.header_image !== current.icon;
  if (!nameChanged && !iconChanged) return null;
  return { name: details.name, icon: details.header_image ?? null };
}
