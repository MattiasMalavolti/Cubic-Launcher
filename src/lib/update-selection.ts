import type { ModUpdateRow } from "./types";

/**
 * The version map the launch receives after the popup, built from the
 * pre-check's `resolved` (D16).
 *
 * `resolved` already carries the **candidate** version for every mod that has
 * an update, so an accepted row needs no work: the map is right as it stands.
 * Only a **refused** row is overwritten, back to the version currently
 * registered in `mod_cache`.
 *
 * Inverting that — writing the candidate for the accepted rows over a map of
 * current versions — would install the updates the user refused, silently and
 * against the list they just read. Hence the direction here, and the rows that
 * are not in `accepted` being the ones that move.
 *
 * "Skip" is the empty `accepted` set: every row falls back to its current
 * version. Mods without a row are untouched either way — they have one
 * version, the one `resolved` names.
 */
export function buildResolvedVersions(
  resolved: Record<string, string>,
  updates: readonly ModUpdateRow[],
  accepted: ReadonlySet<string>,
): Record<string, string> {
  const final: Record<string, string> = { ...resolved };
  for (const row of updates) {
    if (!accepted.has(row.modId)) final[row.modId] = row.currentVersionId;
  }
  return final;
}
