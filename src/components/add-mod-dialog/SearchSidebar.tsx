import { For } from "solid-js";
import { MOD_CATEGORIES, MOD_LOADERS_SEARCH, SORT_OPTIONS } from "./shared";
import { Select } from "../Select";

export function SearchSidebar(props: {
  sortBy: string;
  setSortBy: (value: string) => void;
  selLoaders: Set<string>;
  selEnvs: Set<string>;
  selCategories: Set<string>;
  toggleLoaders: (value: string) => void;
  toggleEnvs: (value: string) => void;
  toggleCategories: (value: string) => void;
  showMoreCats: boolean;
  setShowMoreCats: (value: boolean) => void;
  hasFilters: boolean;
  clearFilters: () => void;
}) {
  return (
    <div class="w-48 shrink-0 border-r border-border overflow-y-auto p-3 space-y-3">
      <div>
        <h4 class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground mb-1.5">Sort by</h4>
        <Select
          value={props.sortBy}
          options={SORT_OPTIONS.map(option => ({ value: option.value, label: option.label }))}
          onChange={value => props.setSortBy(value)}
          class="w-full rounded border border-border bg-input px-2 py-1 text-xs text-foreground"
        />
      </div>

      <div>
        <h4 class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground mb-1.5">Loaders</h4>
        <div class="space-y-1">
          <For each={MOD_LOADERS_SEARCH}>
            {loader => (
              <label class="flex items-center gap-1.5 cursor-pointer text-xs text-foreground hover:text-primary transition-colors">
                <input type="checkbox" checked={props.selLoaders.has(loader)} onChange={() => props.toggleLoaders(loader)} class="accent-accentColor w-3 h-3" />
                {loader.charAt(0).toUpperCase() + loader.slice(1)}
              </label>
            )}
          </For>
        </div>
      </div>

      <div>
        <h4 class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground mb-1.5">Environment</h4>
        <div class="space-y-1">
          <For each={["client", "server"]}>
            {env => (
              <label class="flex items-center gap-1.5 cursor-pointer text-xs text-foreground hover:text-primary transition-colors">
                <input type="checkbox" checked={props.selEnvs.has(env)} onChange={() => props.toggleEnvs(env)} class="accent-accentColor w-3 h-3" />
                {env.charAt(0).toUpperCase() + env.slice(1)}
              </label>
            )}
          </For>
        </div>
      </div>

      <div>
        <h4 class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground mb-1.5">Categories</h4>
        <div class="space-y-1">
          <For each={props.showMoreCats ? MOD_CATEGORIES : MOD_CATEGORIES.slice(0, 8)}>
            {category => (
              <label class="flex items-center gap-1.5 cursor-pointer text-xs text-foreground hover:text-primary transition-colors">
                <input type="checkbox" checked={props.selCategories.has(category)} onChange={() => props.toggleCategories(category)} class="accent-accentColor w-3 h-3" />
                {category.replace("-", " ").replace(/\b\w/g, c => c.toUpperCase())}
              </label>
            )}
          </For>
          <button onClick={() => props.setShowMoreCats(!props.showMoreCats)} class="text-[10px] text-primary hover:underline">
            {props.showMoreCats ? "Show less" : `Show all (${MOD_CATEGORIES.length})`}
          </button>
        </div>
      </div>

      {props.hasFilters && (
        <button
          onClick={props.clearFilters}
          class="w-full rounded border border-dashed border-border px-2 py-1 text-[10px] text-muted-foreground hover:text-foreground hover:border-primary/40 transition-colors"
        >
          Clear all filters
        </button>
      )}
    </div>
  );
}
