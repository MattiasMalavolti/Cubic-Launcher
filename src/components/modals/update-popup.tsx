import { For, Show, createEffect, createSignal } from "solid-js";
import { modIcons, modNames } from "../../store";
import { AlertTriangleIcon, PackageIcon } from "../icons";
import { Modal, ModalHeader } from "./modal-base";
import type { ModUpdateRow } from "../../lib/types";

/**
 * The update popup, presentational on purpose: it receives the pre-check's rows
 * and reports the user's choice, and never talks to the backend. What it knows
 * about the outside world is `modIcons()` and the readable-name cache, the same
 * two lookups every other mod row uses.
 *
 * `onChoose` carries the accepted `modId`s — the set the launch turns into a
 * version map through `buildResolvedVersions`. "Skip" is that callback with an
 * empty set, so "skip" and "nothing checked" are one code path instead of two
 * that have to agree.
 */
export function UpdatePopup(props: {
  updates: ModUpdateRow[];
  versionNumberLookupError?: string | null;
  onChoose: (accepted: ReadonlySet<string>) => void;
  onCancel: () => void;
}) {
  const [accepted, setAccepted] = createSignal<ReadonlySet<string>>(new Set());

  // Every row starts checked: the list is on screen and the action is
  // explicit, so the common case stays one click.
  createEffect(() => setAccepted(new Set(props.updates.map(row => row.modId))));

  const allAccepted = () => props.updates.length > 0 && accepted().size === props.updates.length;
  const someAccepted = () => accepted().size > 0;
  const displayName = (row: ModUpdateRow) => modNames().get(row.projectId) ?? row.modId;
  const iconUrl = (row: ModUpdateRow) => modIcons().get(row.projectId);

  const toggleRow = (modId: string, checked: boolean) => {
    const next = new Set(accepted());
    if (checked) next.add(modId); else next.delete(modId);
    setAccepted(next);
  };

  return (
    <Modal onClose={props.onCancel}>
      <ModalHeader
        title="Updates available"
        onClose={props.onCancel}
        actions={
          <label class="flex items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={allAccepted()}
              ref={el => { createEffect(() => { el.indeterminate = someAccepted() && !allAccepted(); }); }}
              onChange={e => setAccepted(e.currentTarget.checked ? new Set(props.updates.map(row => row.modId)) : new Set())}
              class="h-4 w-4 rounded text-primary"
            />
            <span>Select all</span>
          </label>
        }
      />

      {/* One non-blocking warning for the whole payload, not one per row (D24). */}
      <Show when={props.versionNumberLookupError}>
        <div class="flex items-start gap-2 border-b border-border bg-warning/10 px-6 py-3">
          <AlertTriangleIcon class="mt-0.5 h-4 w-4 shrink-0 text-warning" />
          <p class="text-sm text-warning">
            Current version numbers could not be loaded from Modrinth. The updates below are still accurate.
          </p>
        </div>
      </Show>

      <div class="flex-1 space-y-1 overflow-y-auto px-6 py-3">
        <For each={props.updates}>
          {row => (
            <label class="flex cursor-pointer items-center gap-3 rounded-md px-2 py-2 transition-colors hover:bg-muted/50">
              <div class="flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted">
                <Show when={iconUrl(row)} fallback={<PackageIcon class="h-4 w-4 text-muted-foreground" />}>
                  <img
                    src={iconUrl(row)!}
                    alt={displayName(row)}
                    class="h-8 w-8 object-cover"
                    onError={e => { e.currentTarget.style.display = "none"; }}
                  />
                </Show>
              </div>

              <span class="min-w-0 flex-1 truncate text-sm font-medium text-foreground">{displayName(row)}</span>

              <span class="flex shrink-0 items-center gap-2 text-sm">
                {/* D24: a dash when the current version number is unknown; the arrow and the new version stay. */}
                <span class="text-muted-foreground">{row.currentVersionNumber ?? "—"}</span>
                <span class="text-muted-foreground">&rarr;</span>
                <span class="font-medium text-foreground">{row.candidateVersionNumber}</span>
              </span>

              <input
                type="checkbox"
                checked={accepted().has(row.modId)}
                onChange={e => toggleRow(row.modId, e.currentTarget.checked)}
                class="h-4 w-4 shrink-0 rounded text-primary"
              />
            </label>
          )}
        </For>
      </div>

      <div class="flex justify-end gap-2 border-t border-border px-6 py-4">
        <button
          onClick={() => props.onChoose(new Set())}
          class="rounded-md bg-secondary px-4 py-2 text-sm text-secondary-foreground hover:bg-secondary/80"
        >
          Skip
        </button>
        <button
          onClick={() => props.onChoose(accepted())}
          class="rounded-md bg-primary px-4 py-2 text-sm font-medium text-white hover:bg-brandPurpleHover"
        >
          Update &amp; play
        </button>
      </div>
    </Modal>
  );
}
