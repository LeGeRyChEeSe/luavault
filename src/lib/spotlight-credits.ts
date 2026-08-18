/** One visible credit line in the spotlight header. */
export type CreditLine = {
  key: "spotlight.developer" | "spotlight.publisher" | "combined";
  names: string;
};

/**
 * Turns Steam's optional developer and publisher lists into the lines the
 * spotlight renders. Equality is deliberately positional: Steam's order is
 * retained, so ["A", "B"] and ["B", "A"] remain distinct credits rather
 * than silently changing the behaviour that SPOT-03 shipped.
 */
export function creditLines(developers?: string[], publishers?: string[]): CreditLine[] {
  const hasDevelopers = Boolean(developers?.length);
  const hasPublishers = Boolean(publishers?.length);

  if (hasDevelopers && hasPublishers) {
    const sameCredits = developers!.length === publishers!.length
      && developers!.every((developer, index) => developer === publishers![index]);
    if (sameCredits) return [{ key: "combined", names: developers!.join(", ") }];
    return [
      { key: "spotlight.developer", names: developers!.join(", ") },
      { key: "spotlight.publisher", names: publishers!.join(", ") },
    ];
  }

  if (hasDevelopers) return [{ key: "spotlight.developer", names: developers!.join(", ") }];
  if (hasPublishers) return [{ key: "spotlight.publisher", names: publishers!.join(", ") }];
  return [];
}
