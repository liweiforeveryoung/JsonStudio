<script lang="ts">
  import { createEventDispatcher, tick } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { settingsStore } from '$lib/stores/settings';
  import { t } from '$lib/i18n';
  import { addAncestorPaths, runTreeQuery, type QueryMode } from '$lib/services/treeQuery';
  import type MonacoEditor from './MonacoEditor.svelte';

  type TreeNode = {
    key: string;
    value: unknown;
    type: 'object' | 'array' | 'string' | 'number' | 'boolean' | 'null';
    path: string;
    children?: TreeNode[];
    startOffset: number;
    endOffset: number;
  };

  type QueryExample = {
    query: string;
    result: string;
  };

  type CompletionItem = {
    expr: string;
    pointer: string;
    depth: number;
    label: string;
    kind: 'object' | 'array' | 'primitive';
  };

  type CompletionIndex = {
    jmespath: CompletionItem[];
    jsonpath: CompletionItem[];
    nodeCount: number;
  };

  let { content, editor } = $props<{
    content: string;
    editor: MonacoEditor | null;
  }>();

  const QUERY_DOCS_URL: Record<QueryMode, string> = {
    jmespath: 'https://jmespath.org',
    jsonpath: 'https://datatracker.ietf.org/doc/html/rfc9535',
  };
  const EXAMPLE_DATA = `{
  "people": [
    {"name": "Alice", "age": 20},
    {"name": "Bob",   "age": 30}
  ],
  "meta": {"count": 2}
}`;
  const QUERY_EXAMPLES: Record<QueryMode, QueryExample[]> = {
    jmespath: [
      { query: 'people[0].name', result: '"Alice"' },
      { query: 'people[*].name', result: '["Alice", "Bob"]' },
      { query: 'people[?age > `25`].name', result: '["Bob"]' },
      { query: 'meta.count', result: '2' },
      { query: 'length(people)', result: '2' },
    ],
    jsonpath: [
      { query: '$.people[0].name', result: '"Alice"' },
      { query: '$.people[*].name', result: '["Alice", "Bob"]' },
      { query: '$.people[?(@.age > 25)].name', result: '["Bob"]' },
      { query: '$.meta.count', result: '2' },
      { query: '$..name', result: '["Alice", "Bob"]' },
    ],
  };

  const dispatch = createEventDispatcher<{ toast: { message: string } }>();
  let treeNodes = $state<TreeNode[]>([]);
  let treeError = $state('');
  let isLoading = $state(false);
  let previousContent = $state('');
  let rootData = $state<unknown>(null);
  let selectedPath = $state<string | null>(null);
  let searchQuery = $state('');
  let queryMode = $state<QueryMode>('jmespath');
  let queryError = $state('');
  let queryMatchedRoot = $state(false);
  let queryMatches = $state<Set<string>>(new Set());
  let queryExpandedNodes = $state<Set<string>>(new Set());
  let queryRunId = 0;
  let expandedNodes = $state<Set<string>>(new Set());
  let isAllExpanded = $state(false);
  let helpOpen = $state(false);

  let searchBoxEl: HTMLDivElement;
  let searchInputEl: HTMLInputElement;

  const nodeElMap = new Map<string, HTMLElement>();

  const COMPLETION_NODE_LIMIT = 50_000;
  const SUGGEST_LIMIT = 30;
  const SUGGEST_DEBOUNCE_MS = 80;

  let completionIndex = $state<CompletionIndex | null>(null);
  let suggestions = $state<CompletionItem[]>([]);
  let isSuggestOpen = $state(false);
  let activeSuggestIndex = $state(0);
  let previewPointer = $state<string | null>(null);
  let previewPath = $state<string | null>(null);
  let suggestTimer: ReturnType<typeof setTimeout> | null = null;
  let lastFilterMode = $state<QueryMode>('jmespath');
  let lastFilterInput = $state('');
  let lastFiltered = $state<CompletionItem[]>([]);

  function registerTreeNodeEl(el: HTMLElement, path: string) {
    nodeElMap.set(path, el);
    return {
      destroy() {
        const current = nodeElMap.get(path);
        if (current === el) nodeElMap.delete(path);
      },
    };
  }

  function scrollSuggestItemIntoView(index: number) {
    requestAnimationFrame(() => {
      const container = document.querySelector('.json-tree-suggest');
      const items = container?.querySelectorAll('.json-tree-suggest-item');
      const item = items?.[index] as HTMLButtonElement | undefined;
      if (item && container) {
        const containerRect = container.getBoundingClientRect();
        const itemRect = item.getBoundingClientRect();

        if (itemRect.top < containerRect.top) {
          item.scrollIntoView({ block: 'start', behavior: 'smooth' });
        } else if (itemRect.bottom > containerRect.bottom) {
          item.scrollIntoView({ block: 'end', behavior: 'smooth' });
        }
      }
    });
  }

  // Build tree when content changes
  $effect(() => {
    if (content !== previousContent) {
      previousContent = content;
      buildTree();
    }
  });

  $effect(() => {
    const query = searchQuery.trim();
    const data = rootData;
    const nodes = treeNodes;
    void updateQueryMatches(queryMode, query, data, nodes);
  });

  $effect(() => {
    const data = rootData;
    const nodes = treeNodes;
    if (data == null || nodes.length === 0) {
      completionIndex = null;
      suggestions = [];
      isSuggestOpen = false;
      activeSuggestIndex = 0;
      lastFilterInput = '';
      lastFiltered = [];
      return;
    }
    completionIndex = buildCompletionIndex(nodes, data);
  });

  $effect(() => {
    scheduleSuggestUpdate(queryMode, searchQuery, completionIndex);
  });

  $effect(() => {
    const _q = searchQuery;
    const _m = queryMode;
    if (_q === undefined || _m === undefined) return;
    previewPointer = null;
    previewPath = null;
  });

  $effect(() => {
    if (isSuggestOpen) return;
    previewPointer = null;
    previewPath = null;
  });

  $effect(() => {
    const pointer = previewPointer;
    const open = isSuggestOpen;
    const nodes = treeNodes;
    if (!open || !pointer || nodes.length === 0) return;

    previewPath = pointer;

    const nextExpanded = new Set(expandedNodes);
    addAncestorPaths(pointer, nextExpanded);
    if (nextExpanded.size !== expandedNodes.size) {
      expandedNodes = nextExpanded;
      isAllExpanded = false;
    }

    let cancelled = false;
    void (async () => {
      await tick();
      if (cancelled) return;

      let el = nodeElMap.get(pointer);
      if (!el) {
        await tick();
        if (cancelled) return;
        el = nodeElMap.get(pointer);
      }

      el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    })();

    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const open = isSuggestOpen;
    if (!open) return;

    const onGlobalClickCapture = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && searchBoxEl?.contains(target)) return;
      isSuggestOpen = false;
    };

    window.addEventListener('click', onGlobalClickCapture, true);
    return () => window.removeEventListener('click', onGlobalClickCapture, true);
  });

  function buildCompletionIndex(nodes: TreeNode[], data: unknown): CompletionIndex {
    const jmespath: CompletionItem[] = [];
    const jsonpath: CompletionItem[] = [];
    let nodeCount = 0;

    const walk = (list: TreeNode[]) => {
      for (const node of list) {
        if (nodeCount >= COMPLETION_NODE_LIMIT) return;
        nodeCount++;

        const depth = node.path ? node.path.split('/').length - 1 : 0;
        const kind: CompletionItem['kind'] =
          node.type === 'object' ? 'object' : node.type === 'array' ? 'array' : 'primitive';

        jmespath.push({
          expr: pointerToJmesPath(node.path, data),
          pointer: node.path,
          depth,
          label: node.key,
          kind,
        });
        jsonpath.push({
          expr: pointerToJsonPath(node.path, data),
          pointer: node.path,
          depth,
          label: node.key,
          kind,
        });

        if (node.children?.length) {
          walk(node.children);
        }
      }
    };

    walk(nodes);

    const byDepthThenExpr = (a: CompletionItem, b: CompletionItem) =>
      a.depth - b.depth || a.expr.localeCompare(b.expr);

    jmespath.sort(byDepthThenExpr);
    jsonpath.sort(byDepthThenExpr);

    return { jmespath, jsonpath, nodeCount };
  }

  function normalizeJsonPathForMatch(value: string): string {
    let s = value.trim();
    if (s.startsWith('$')) s = s.slice(1);
    if (s.startsWith('.')) s = s.slice(1);
    return s;
  }

  function matchesExpr(mode: QueryMode, expr: string, input: string): boolean {
    const raw = input.trim();
    if (!raw) return false;

    if (expr.startsWith(raw)) return true;

    if (mode === 'jsonpath') {
      const inNorm = normalizeJsonPathForMatch(raw);
      const exNorm = normalizeJsonPathForMatch(expr);
      if (inNorm && exNorm.startsWith(inNorm)) return true;
      if (inNorm.length >= 2 && exNorm.includes(inNorm)) return true;
    }

    return raw.length >= 2 ? expr.includes(raw) : false;
  }

  function scheduleSuggestUpdate(
    mode: QueryMode,
    rawInput: string,
    index: CompletionIndex | null
  ) {
    if (suggestTimer) clearTimeout(suggestTimer);
    suggestTimer = setTimeout(() => {
      updateSuggestions(mode, rawInput, index);
    }, SUGGEST_DEBOUNCE_MS);
  }

  function updateSuggestions(mode: QueryMode, rawInput: string, index: CompletionIndex | null) {
    const input = rawInput.trim();
    if (!index || !input) {
      suggestions = [];
      isSuggestOpen = false;
      activeSuggestIndex = 0;
      lastFilterInput = '';
      lastFiltered = [];
      return;
    }

    const all = mode === 'jsonpath' ? index.jsonpath : index.jmespath;
    const canReuse = lastFilterMode === mode && input.startsWith(lastFilterInput) && lastFiltered.length > 0;
    const source = canReuse ? lastFiltered : all;
    const next: CompletionItem[] = [];

    for (const item of source) {
      if (matchesExpr(mode, item.expr, input)) {
        next.push(item);
        if (next.length >= SUGGEST_LIMIT) break;
      }
    }

    suggestions = next;
    isSuggestOpen = next.length > 0;
    activeSuggestIndex = 0;
    lastFilterMode = mode;
    lastFilterInput = input;
    lastFiltered = next;
  }

  function applySuggestion(item: CompletionItem) {
    const current = searchQuery;
    const cursorAtEnd = (searchInputEl?.selectionStart ?? current.length) === current.length;
    const nextValue = cursorAtEnd && item.expr.startsWith(current) ? item.expr : item.expr;
    searchQuery = nextValue;
    isSuggestOpen = false;
    activeSuggestIndex = 0;
    previewPointer = null;
    previewPath = null;
    setTimeout(() => {
      try {
        searchInputEl?.focus();
        searchInputEl?.setSelectionRange(nextValue.length, nextValue.length);
      } catch (_) {}
    }, 0);
  }

  function handleSearchKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      isSuggestOpen = false;
      previewPointer = null;
      previewPath = null;
      return;
    }

    if (event.key === 'ArrowDown') {
      if (!isSuggestOpen && suggestions.length > 0) isSuggestOpen = true;
      if (!isSuggestOpen || suggestions.length === 0) return;
      event.preventDefault();
      if (activeSuggestIndex < suggestions.length - 1) {
        activeSuggestIndex++;
      }
      previewPointer = suggestions[activeSuggestIndex]?.pointer ?? null;
      scrollSuggestItemIntoView(activeSuggestIndex);
      return;
    }

    if (event.key === 'ArrowUp') {
      if (!isSuggestOpen && suggestions.length > 0) isSuggestOpen = true;
      if (!isSuggestOpen || suggestions.length === 0) return;
      event.preventDefault();
      if (activeSuggestIndex > 0) {
        activeSuggestIndex--;
      }
      previewPointer = suggestions[activeSuggestIndex]?.pointer ?? null;
      scrollSuggestItemIntoView(activeSuggestIndex);
      return;
    }

    if (event.key === 'Enter' || event.key === 'Tab') {
      if (!isSuggestOpen || suggestions.length === 0) return;
      event.preventDefault();
      const item = suggestions[activeSuggestIndex];
      if (item) applySuggestion(item);
    }
  }

  async function buildTree() {
    if (!content.trim()) {
      treeNodes = [];
      treeError = '';
      rootData = null;
      queryError = '';
      queryMatchedRoot = false;
      isAllExpanded = false;
      return;
    }

    isLoading = true;
    treeError = '';

    try {
      const { parse } = await import('@mischnic/json-sourcemap');

      // Try to parse as standard JSON first
      let parsed;
      try {
        parsed = parse(content, undefined, { dialect: 'JSON' });
      } catch (jsonError) {
        // If standard JSON fails, try JSON5
        try {
          parsed = parse(content, undefined, { dialect: 'JSON5' });
        } catch (json5Error) {
          // If both fail, throw the original error
          throw jsonError;
        }
      }

      rootData = parsed.data;
      const nodes = parseToTree(parsed.data, parsed.pointers, '');
      treeNodes = nodes;
      
      // Auto-expand first level
      if (nodes.length > 0) {
        expandedNodes = new Set(nodes.map(n => n.path));
      }
      isAllExpanded = false;
    } catch (e) {
      treeError = e instanceof Error ? e.message : 'Failed to parse JSON';
      treeNodes = [];
      rootData = null;
      isAllExpanded = false;
    } finally {
      isLoading = false;
    }
  }

  function parseToTree(data: unknown, pointers: any, parentPath: string): TreeNode[] {
    const nodes: TreeNode[] = [];

    if (data === null) {
      return [];
    }

    if (Array.isArray(data)) {
      data.forEach((item, index) => {
        const path = parentPath ? `${parentPath}/${index}` : `/${index}`;
        const pointerInfo = pointers[path];
        
        const node: TreeNode = {
          key: `[${index}]`,
          value: item,
          type: getValueType(item),
          path,
          startOffset: pointerInfo?.value?.pos ?? 0,
          endOffset: pointerInfo?.valueEnd?.pos ?? 0,
        };

        if (node.type === 'object' || node.type === 'array') {
          node.children = parseToTree(item, pointers, path);
        }

        nodes.push(node);
      });
    } else if (typeof data === 'object' && data !== null) {
      Object.entries(data).forEach(([key, value]) => {
        const path = parentPath ? `${parentPath}/${encodePointerSegment(key)}` : `/${encodePointerSegment(key)}`;
        const pointerInfo = pointers[path];

        const node: TreeNode = {
          key,
          value,
          type: getValueType(value),
          path,
          startOffset: pointerInfo?.value?.pos ?? 0,
          endOffset: pointerInfo?.valueEnd?.pos ?? 0,
        };

        if (node.type === 'object' || node.type === 'array') {
          node.children = parseToTree(value, pointers, path);
        }

        nodes.push(node);
      });
    }

    return nodes;
  }

  function encodePointerSegment(segment: string): string {
    return segment.replace(/~/g, '~0').replace(/\//g, '~1');
  }

  function decodePointerSegment(segment: string): string {
    return segment.replace(/~1/g, '/').replace(/~0/g, '~');
  }

  function pointerToJmesPath(path: string, data: unknown): string {
    if (!path || path === '/') return '';
    const segments = path.split('/').slice(1).map(decodePointerSegment);
    let result = '';
    let current = data;
    for (const segment of segments) {
      const isArrayIndex = Array.isArray(current) && /^[0-9]+$/.test(segment);
      if (isArrayIndex) {
        result += `[${segment}]`;
        current = (current as any)[Number(segment)];
        continue;
      }
      const isIdentifier = /^[A-Za-z_][0-9A-Za-z_]*$/.test(segment);
      const token = isIdentifier ? segment : JSON.stringify(segment);
      result = result ? `${result}.${token}` : token;
      if (current && typeof current === 'object' && !Array.isArray(current)) {
        current = (current as Record<string, unknown>)[segment];
      } else {
        current = undefined;
      }
    }
    return result;
  }

  function pointerToJsonPath(path: string, data: unknown): string {
    if (!path || path === '/') return '$';

    const segments = path.split('/').slice(1).map(decodePointerSegment);
    let result = '$';
    let current = data;

    for (const segment of segments) {
      const isArrayIndex = Array.isArray(current) && /^[0-9]+$/.test(segment);
      if (isArrayIndex) {
        result += `[${segment}]`;
        current = (current as any)[Number(segment)];
        continue;
      }

      const isIdentifier = /^[A-Za-z_$][0-9A-Za-z_$]*$/.test(segment);
      result += isIdentifier ? `.${segment}` : `[${JSON.stringify(segment)}]`;

      if (current && typeof current === 'object' && !Array.isArray(current)) {
        current = (current as Record<string, unknown>)[segment];
      } else {
        current = undefined;
      }
    }

    return result;
  }

  function getValueType(value: unknown): TreeNode['type'] {
    if (value === null) return 'null';
    if (Array.isArray(value)) return 'array';
    if (typeof value === 'object') return 'object';
    if (typeof value === 'string') return 'string';
    if (typeof value === 'number') return 'number';
    if (typeof value === 'boolean') return 'boolean';
    return 'string';
  }

  function formatValue(node: TreeNode): string {
    if (node.type === 'string') {
      const str = String(node.value);
      if (str.length > 50) {
        return str.slice(0, 47) + '...';
      }
      return str;
    }
    if (node.type === 'null') {
      return 'null';
    }
    if (node.type === 'boolean') {
      return String(node.value);
    }
    if (node.type === 'number') {
      // Extract the raw number text from the source to avoid precision loss
      // for integers exceeding Number.MAX_SAFE_INTEGER
      if (node.startOffset < node.endOffset && node.endOffset <= content.length) {
        const raw = content.slice(node.startOffset, node.endOffset).trim();
        if (raw && /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(raw)) {
          return raw;
        }
      }
      return String(node.value);
    }
    return '';
  }

  function hasChildren(node: TreeNode): boolean {
    return (node.type === 'object' || node.type === 'array') && (node.children?.length ?? 0) > 0;
  }

  function getChildCount(node: TreeNode): number {
    return node.children?.length || 0;
  }

  function toggleNode(node: TreeNode) {
    if (expandedNodes.has(node.path)) {
      expandedNodes.delete(node.path);
    } else {
      expandedNodes.add(node.path);
    }
    expandedNodes = new Set(expandedNodes);
    isAllExpanded = false;
  }

  function selectNode(node: TreeNode) {
    const editorInstance = editor?.getEditorInstance();
    const model = editorInstance?.getModel();
    if (!editorInstance || !model) return;

    selectedPath = node.path;

    const endOffset = node.endOffset <= node.startOffset ? node.startOffset + 1 : node.endOffset;
    const start = model.getPositionAt(node.startOffset);
    const end = model.getPositionAt(endOffset);
    
    editorInstance.setSelection({
      startLineNumber: start.lineNumber,
      startColumn: start.column,
      endLineNumber: end.lineNumber,
      endColumn: end.column
    });
    
    editorInstance.revealPositionInCenter(start);
    editorInstance.focus();
  }

  function handleNodeKeydown(event: KeyboardEvent, node: TreeNode) {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    selectNode(node);
  }

  function expandAll() {
    const allPaths = new Set<string>();
    const collectPaths = (nodes: TreeNode[]) => {
      nodes.forEach(node => {
        allPaths.add(node.path);
        if (node.children) {
          collectPaths(node.children);
        }
      });
    };
    collectPaths(treeNodes);
    expandedNodes = allPaths;
    isAllExpanded = true;
  }

  function collapseAll() {
    expandedNodes = new Set();
    isAllExpanded = false;
  }

  function getRawValue(node: TreeNode): string {
    if (node.startOffset < node.endOffset && node.endOffset <= content.length) {
      return content.slice(node.startOffset, node.endOffset);
    }
    return '';
  }

  async function copyEntry(node: TreeNode) {
    const queryPath = queryMode === 'jsonpath'
      ? pointerToJsonPath(node.path, rootData)
      : pointerToJmesPath(node.path, rootData);
    const dotPath = queryPath || node.key;
    const rawSource = getRawValue(node);
    const valueText = rawSource || (JSON.stringify(node.value) ?? 'null');
    const entryText = `${dotPath}: ${valueText}`;

    try {
      await navigator.clipboard.writeText(entryText);
      dispatch('toast', { message: $t('treeView.pathValueCopied') });
    } catch (e) {}
  }

  function getTypeIcon(type: TreeNode['type']): string {
    switch (type) {
      case 'object': return '{}';
      case 'array': return '[]';
      case 'string': return 'str';
      case 'number': return 'num';
      case 'boolean': return 'bool';
      case 'null': return '∅';
      default: return '';
    }
  }

  function isLastChild(nodes: TreeNode[], index: number): boolean {
    return index === nodes.length - 1;
  }

  async function updateQueryMatches(
    mode: QueryMode,
    query: string,
    data: unknown,
    nodes: TreeNode[]
  ) {
    const runId = ++queryRunId;
    if (!query || data == null || nodes.length === 0) {
      queryMatches = new Set();
      queryExpandedNodes = new Set();
      queryError = '';
      queryMatchedRoot = false;
      return;
    }

    const { matches, expanded, error, matchedRoot } = await runTreeQuery({
      mode,
      query,
      data,
      nodes,
    });
    if (runId !== queryRunId) return;
    queryMatches = matches;
    queryExpandedNodes = matchedRoot
      ? new Set([...expanded, ...nodes.map((node) => node.path)])
      : expanded;
    queryError = error;
    queryMatchedRoot = matchedRoot;
  }

  function getQueryModeLabel(mode: QueryMode): string {
    return mode === 'jsonpath' ? 'JSONPath' : 'JMESPath';
  }

  function getQueryDocsUrl(mode: QueryMode): string {
    return QUERY_DOCS_URL[mode];
  }

  function getQueryExamples(mode: QueryMode): QueryExample[] {
    return QUERY_EXAMPLES[mode];
  }

  function hideHelp() {
    helpOpen = false;
  }
</script>

<svelte:window onclick={hideHelp} />

<div class="json-tree-panel">
  <!-- Header -->
  <div class="json-tree-header">
    <div class="flex items-center gap-2">
      <div class="json-tree-icon">
        <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="9" y="2" width="6" height="6" rx="1"/>
          <rect x="3" y="14" width="6" height="6" rx="1"/>
          <rect x="15" y="14" width="6" height="6" rx="1"/>
          <path d="M12 8v3"/>
          <path d="M12 11h-6"/>
          <path d="M12 11h6"/>
          <path d="M6 14v-3"/>
          <path d="M18 14v-3"/>
        </svg>
      </div>
      <div style="font-size: 14px;" class="font-semibold text-(--text-primary)">{$t('treeView.title')}</div>
    </div>
    <button
      class="json-tree-close-btn"
      onclick={() => settingsStore.updateSetting('showTreeView', false)}
      title={$t('treeView.hide')}
      type="button"
    >
      <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 6L6 18M6 6l12 12"/>
      </svg>
    </button>
  </div>

  <!-- Toolbar -->
<div class="json-tree-toolbar">
    <div class="json-tree-search-box" bind:this={searchBoxEl}>
      <svg class="json-tree-search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="11" cy="11" r="8"/>
        <path d="m21 21-4.35-4.35"/>
      </svg>
      <input
        class="json-tree-search-input"
        placeholder={queryMode === 'jsonpath' ? $t('treeView.searchPlaceholderJsonpath') : $t('treeView.searchPlaceholder')}
        value={searchQuery}
        oninput={(e) => { searchQuery = e.currentTarget.value; }}
        onfocus={() => scheduleSuggestUpdate(queryMode, searchQuery, completionIndex)}
        onblur={() => { isSuggestOpen = false; }}
        onkeydown={handleSearchKeydown}
        bind:this={searchInputEl}
        spellcheck="false"
      />
      {#if searchQuery}
        <button
          class="json-tree-clear-btn"
          onclick={() => { searchQuery = ''; isSuggestOpen = false; }}
          title={$t('treeView.clearQuery')}
          type="button"
        >
          <svg class="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6L6 18M6 6l12 12"/>
          </svg>
        </button>
      {/if}

      {#if isSuggestOpen}
        <div class="json-tree-suggest" role="listbox" aria-label="query suggestions">
          {#each suggestions as item, i}
            <button
              type="button"
              class="json-tree-suggest-item"
              class:is-active={i === activeSuggestIndex}
              role="option"
              aria-selected={i === activeSuggestIndex}
              title={item.expr}
              onmousedown={(e) => {
                e.preventDefault();
                applySuggestion(item);
              }}
            >
              <span class="json-tree-suggest-expr">{item.expr}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="json-tree-toolbar-actions">
      <select
        class="json-tree-mode-select"
        bind:value={queryMode}
        aria-label={$t('treeView.queryMode')}
        title={$t('treeView.queryMode')}
      >
        <option value="jmespath">{$t('treeView.modeJmespath')}</option>
        <option value="jsonpath">{$t('treeView.modeJsonpath')}</option>
      </select>

      <div
        class="json-tree-help"
        role="group"
        aria-label={`${getQueryModeLabel(queryMode)} Help`}
      >
        <button
          class="json-tree-help-btn"
          class:is-active={helpOpen}
          onclick={(e) => { e.stopPropagation(); helpOpen = !helpOpen; }}
          type="button"
          title={`${getQueryModeLabel(queryMode)} ${$t('treeView.syntaxGuide')}`}
          aria-expanded={helpOpen}
          aria-controls={`${queryMode}-help`}
        >
          <svg class="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
            <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1Zm-.75 3.75a.75.75 0 1 1 1.5 0 .75.75 0 0 1-1.5 0ZM7.25 7.5a.75.75 0 0 1 .75-.75h.01a.75.75 0 0 1 .74.75v3.25h.25a.5.5 0 0 1 0 1h-1.5a.5.5 0 0 1 0-1h.25V8.25h-.01a.75.75 0 0 1-.49-.75Z"/>
          </svg>
        </button>
        
        {#if helpOpen}
          <div
            class="json-tree-help-popover"
            id={`${queryMode}-help`}
            role="dialog"
            aria-label={`${getQueryModeLabel(queryMode)} Help`}
            tabindex="-1"
            onclick={(e) => e.stopPropagation()}
            onkeydown={(e) => e.key === 'Escape' && hideHelp()}
          >
            <div class="json-tree-help-header">
              <span class="json-tree-help-title">{getQueryModeLabel(queryMode)} {$t('treeView.cheatSheet')}</span>
              <a 
                href={getQueryDocsUrl(queryMode)}
                target="_blank" 
                rel="noopener noreferrer" 
                class="json-tree-help-link"
                onclick={async (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  const url = getQueryDocsUrl(queryMode);
                  try {
                    await openUrl(url);
                  } catch (err) {
                    console.error('Failed to open link:', err);
                    window.open(url, '_blank');
                  }
                }}
              >
                {$t('treeView.docs')}
                <svg class="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
                  <polyline points="15 3 21 3 21 9"/>
                  <line x1="10" y1="14" x2="21" y2="3"/>
                </svg>
              </a>
            </div>
            
            <div class="json-tree-help-content">
              <div class="json-tree-help-section">
                <div class="json-tree-help-label">{$t('treeView.exampleData')}</div>
                <div class="json-tree-help-code-wrapper">
                  <button
                    class="json-tree-copy-code-btn"
                    onclick={(e) => {
                      e.stopPropagation();
                      navigator.clipboard.writeText(EXAMPLE_DATA);
                      dispatch('toast', { message: $t('treeView.exampleCopied') });
                    }}
                    title={$t('treeView.copyExample')}
                    type="button"
                  >
                    <svg class="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <rect x="9" y="9" width="13" height="13" rx="2"></rect>
                      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
                    </svg>
                  </button>
                  <pre class="json-tree-help-code-block">{EXAMPLE_DATA}</pre>
                </div>
              </div>

              <div class="json-tree-help-section">
                <div class="json-tree-help-label">{$t('treeView.exampleQueries')}</div>
                <div class="json-tree-help-grid">
                  {#each getQueryExamples(queryMode) as example}
                    <div class="help-item">
                      <div class="help-query">{example.query}</div>
                      <div class="help-desc">{example.result}</div>
                    </div>
                  {/each}
                </div>
              </div>
            </div>
          </div>
        {/if}
      </div>

      <button 
        class="json-tree-action-btn" 
        onclick={isAllExpanded ? collapseAll : expandAll} 
        disabled={treeNodes.length === 0} 
        title={isAllExpanded ? $t('treeView.collapseAll') : $t('treeView.expandAll')}
      >
        {#if isAllExpanded}
          <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m17 11-5-5-5 5M17 18l-5-5-5 5"/>
          </svg>
        {:else}
          <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m7 13 5 5 5-5M7 6l5 5 5-5"/>
          </svg>
        {/if}
      </button>
    </div>
  </div>

  {#if queryError}
    <div class="json-tree-query-error" role="alert">
      {queryError}
    </div>
  {:else if queryMatchedRoot}
    <div class="json-tree-query-info" role="status">
      {$t('treeView.rootMatched')}
    </div>
  {/if}

  <!-- Tree Content -->
  <div class="json-tree-content">
    {#if isLoading}
      <div class="json-tree-empty">
        <svg class="w-8 h-8 text-(--text-secondary) animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
        </svg>
        <div class="text-xs text-(--text-secondary) mt-2">{$t('treeView.parsing')}</div>
      </div>
    {:else if treeError}
      <div class="json-tree-empty">
        <svg class="w-12 h-12 text-(--text-secondary) opacity-40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
          <line x1="12" y1="9" x2="12" y2="13"/>
          <line x1="12" y1="17" x2="12.01" y2="17"/>
        </svg>
        <div class="text-xs font-medium text-(--text-primary) mt-3 opacity-70">{$t('treeView.invalidJson')}</div>
      </div>
    {:else if treeNodes.length === 0}
      <div class="json-tree-empty">
        <svg class="w-12 h-12 text-(--text-secondary) opacity-30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="9" y="2" width="6" height="6" rx="1"/>
          <rect x="3" y="14" width="6" height="6" rx="1"/>
          <rect x="15" y="14" width="6" height="6" rx="1"/>
          <path d="M12 8v3"/>
          <path d="M12 11h-6"/>
          <path d="M12 11h6"/>
          <path d="M6 14v-3"/>
          <path d="M18 14v-3"/>
        </svg>
        <div class="text-xs font-medium text-(--text-primary) mt-3 opacity-60">{$t('treeView.noData')}</div>
        <div class="text-xs text-(--text-secondary) mt-1 opacity-50">{$t('treeView.noDataHint')}</div>
      </div>
    {:else}
      {#snippet renderNode(node: TreeNode, depth: number, isLast: boolean, parentLines: boolean[])}
        {@const hasChild = hasChildren(node)}
        {@const isExpanded = expandedNodes.has(node.path) || queryExpandedNodes.has(node.path)}
        {@const isSelected = selectedPath === node.path}
        {@const isPreview = !isSelected && previewPath === node.path}
        {@const isMatched = queryMatches.has(node.path)}
        {@const childCount = getChildCount(node)}
        {@const showValue = node.type !== 'object' && node.type !== 'array'}
        
        <div
          class="tree-node"
          class:tree-node-selected={isSelected}
          class:tree-node-preview={isPreview}
          class:tree-node-matched={isMatched}
        >
          <div 
            class="tree-node-content"
            onclick={() => selectNode(node)}
            onkeydown={(e) => handleNodeKeydown(e, node)}
            role="button"
            tabindex="0"
            data-pointer={node.path}
            use:registerTreeNodeEl={node.path}
          >
            <!-- Tree Lines -->
            {#if depth > 0}
              <div class="tree-lines">
                {#each parentLines as hasLine}
                  <div class="tree-line-segment">
                    {#if hasLine}
                      <div class="tree-line-vertical"></div>
                    {/if}
                  </div>
                {/each}
                
                <!-- Current level connector -->
                <div class="tree-connector">
                  {#if isLast}
                    <div class="tree-line-corner-last"></div>
                  {:else}
                    <div class="tree-line-corner"></div>
                  {/if}
                </div>
              </div>
            {/if}

            <!-- Expand/Collapse Button -->
            <div class="tree-toggle-area">
              {#if hasChild}
                <button
                  class="tree-toggle-btn"
                  onclick={(e) => { e.stopPropagation(); toggleNode(node); }}
                  aria-label={isExpanded ? `Collapse ${node.key}` : `Expand ${node.key}`}
                  title={isExpanded ? `Collapse ${node.key}` : `Expand ${node.key}`}
                  type="button"
                >
                  <svg 
                    class="w-3 h-3 transition-transform duration-150 {isExpanded ? 'rotate-90' : ''}" 
                    viewBox="0 0 24 24" 
                    fill="currentColor"
                  >
                    <path d="M8.59 16.59L13.17 12 8.59 7.41 10 6l6 6-6 6-1.41-1.41z"/>
                  </svg>
                </button>
              {/if}
            </div>

            <!-- Type Icon -->
            <div class="tree-type-icon tree-type-{node.type}">
              {getTypeIcon(node.type)}
            </div>

            <!-- Key-Value Pair -->
            <div class="tree-key-value">
              <span class="tree-key">{node.key}</span>{#if showValue}<span class="tree-colon">:</span><span class="tree-value tree-value-{node.type}">{formatValue(node)}</span>{:else if hasChild}<span class="tree-child-count">({childCount})</span>{/if}
            </div>

            <!-- Copy Path Button -->
            <button
              class="tree-copy-btn"
              onclick={(e) => { e.stopPropagation(); copyEntry(node); }}
              title={$t('treeView.copyPathValue')}
              type="button"
            >
              <svg class="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="9" y="9" width="13" height="13" rx="2"></rect>
                <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
              </svg>
            </button>
          </div>
        </div>

        {#if hasChild && isExpanded && node.children}
          {#if depth === 0}
            <!-- Root node children don't need parent vertical lines -->
            {#each node.children as child, i}
              {@render renderNode(child, depth + 1, isLastChild(node.children!, i), [])}
            {/each}
          {:else}
            <!-- Non-root node children need parent vertical lines -->
            {@const newParentLines = [...parentLines, !isLast]}
            {#each node.children as child, i}
              {@render renderNode(child, depth + 1, isLastChild(node.children!, i), newParentLines)}
            {/each}
          {/if}
        {/if}
      {/snippet}

      <div class="tree-list">
        {#each treeNodes as node, i}
          {@render renderNode(node, 0, isLastChild(treeNodes, i), [])}
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  /* Query Toolbar */
  .json-tree-toolbar-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .json-tree-mode-select {
    height: 22px;
    min-width: 90px;
    padding: 0 22px 0 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    outline: none;
    appearance: none;
    background-image:
      linear-gradient(45deg, transparent 50%, var(--text-secondary) 50%),
      linear-gradient(135deg, var(--text-secondary) 50%, transparent 50%);
    background-position:
      calc(100% - 13px) 9px,
      calc(100% - 8px) 9px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  .json-tree-mode-select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-glow);
  }

  .json-tree-search-box {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 6px;
    transition: all 0.2s ease;
    height: 28px;
    min-width: 0;
    position: relative;
    overflow: visible;
  }

  .json-tree-search-box:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-glow);
  }

  .json-tree-search-icon {
    width: 14px;
    height: 14px;
    color: var(--text-secondary);
    flex-shrink: 0;
  }

  .json-tree-search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    font-size: 12px;
    font-family: 'JetBrains Mono', monospace;
    color: var(--text-primary);
    min-width: 0;
  }

  .json-tree-search-input::placeholder {
    color: var(--text-secondary);
    opacity: 0.5;
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
  }

  .json-tree-clear-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    color: var(--text-secondary);
    background: var(--bg-tertiary);
    border: none;
    cursor: pointer;
    transition: all 0.15s ease;
    flex-shrink: 0;
  }

  .json-tree-clear-btn:hover {
    background: var(--text-secondary);
    color: var(--bg-primary);
  }

  .json-tree-suggest {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    max-height: 220px;
    overflow: auto;
    padding: 6px;
    border-radius: 8px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    box-shadow: 0 6px 22px rgba(0, 0, 0, 0.25);
    z-index: 1500;
  }

  .json-tree-suggest-item {
    width: 100%;
    display: flex;
    align-items: center;
    padding: 6px 8px;
    border-radius: 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--text-primary);
    transition: background 0.12s ease;
  }

  .json-tree-suggest-item:hover,
  .json-tree-suggest-item.is-active {
    background: var(--bg-secondary);
  }

  .json-tree-suggest-expr {
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    line-height: 1.4;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .json-tree-query-error {
    margin: 0 10px;
    padding: 6px 10px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-secondary));
    color: var(--text-primary);
    font-size: 11px;
    border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border));
  }

  .json-tree-query-info {
    margin: 0 10px;
    padding: 6px 10px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--bg-tertiary) 75%, var(--bg-secondary));
    color: var(--text-secondary);
    font-size: 11px;
    border: 1px solid var(--border);
  }

  /* Help Button & Popover */
  .json-tree-help {
    position: relative;
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  .json-tree-help-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border-radius: 50%;
    color: var(--text-secondary);
    opacity: 0.4;
    background: transparent;
    border: none;
    cursor: pointer;
    transition: all 0.15s ease;
    overflow: visible;
  }

  .json-tree-help-btn:hover,
  .json-tree-help-btn.is-active {
    opacity: 0.8;
    color: var(--text-primary);
  }

  .json-tree-help-popover {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    width: 380px;
    max-width: calc(100vw - 20px);
    max-height: 400px;
    padding: 0;
    border-radius: 8px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.25);
    color: var(--text-primary);
    font-size: 12px;
    overflow: hidden;
    z-index: 2000;
    display: flex;
    flex-direction: column;
  }

  .json-tree-help-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
  }

  .json-tree-help-title {
    font-weight: 600;
    color: var(--text-primary);
  }

  .json-tree-help-link {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
  }

  .json-tree-help-link:hover {
    text-decoration: underline;
  }

  .json-tree-help-content {
    padding: 14px;
    overflow-y: auto;
  }

  .json-tree-help-section {
    margin-bottom: 16px;
  }

  .json-tree-help-section:last-child {
    margin-bottom: 0;
  }

  .json-tree-help-label {
    font-size: 10px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    margin-bottom: 6px;
  }

  .json-tree-help-code-wrapper {
    position: relative;
  }

  .json-tree-copy-code-btn {
    position: absolute;
    top: 6px;
    right: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 4px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    color: var(--text-secondary);
    cursor: pointer;
    opacity: 0;
    transition: all 0.15s ease;
  }

  .json-tree-help-code-wrapper:hover .json-tree-copy-code-btn {
    opacity: 1;
  }

  .json-tree-copy-code-btn:hover {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-color: var(--accent);
  }

  .json-tree-help-code-block {
    font-family: 'JetBrains Mono', monospace;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 10px;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-primary);
    overflow-x: auto;
  }

  .json-tree-help-grid {
    display: grid;
    gap: 1px;
    background: var(--border);
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }

  .help-item {
    display: grid;
    grid-template-columns: 1fr 1fr;
    background: var(--bg-primary);
    padding: 8px 10px;
  }

  .help-item:hover {
    background: var(--bg-secondary);
  }

  .help-query {
    font-family: 'JetBrains Mono', monospace;
    color: var(--accent);
    font-size: 11px;
  }

  .help-desc {
    color: var(--text-secondary);
    font-size: 11px;
    text-align: right;
  }

</style>
