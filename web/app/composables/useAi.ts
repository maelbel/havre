// AI: an app you add from the App library. Mirrors `crates/server/src/ai.rs`.
//
// `status.installed`: the AI app is in the dock. `status.enabled`: installed
// *and* set up (a model chosen) — what the other apps check before showing
// their AI parts (summaries, the brief, Ask in Ctrl+K).
import type { AgendaEvent } from './useAgenda'
import type { Person } from './useMail'
import type { NoteSummary } from './useNotes'
import type { EventLink, PersonLink, Task } from './useTasks'

export type AiProvider = 'ollama' | 'openai' | 'anthropic' | 'claude-code'

export interface AiStatus {
  installed?: boolean
  enabled: boolean
  provider?: AiProvider
  model?: string
  local?: boolean
  /** Search by meaning is set up. */
  search: boolean
  indexed?: number
  total?: number
  index_error?: string | null
  /** When today's brief was written (unix seconds). */
  brief_at?: number | null
}

export interface AiActivity {
  id: number
  at: number
  source: string
  action: string
  detail: string
}

export function fetchAiActivity(limit = 100) {
  return $fetch<AiActivity[]>('/api/ai/activity', { query: { limit } })
}

/** Notes in the activity log what you confirmed. */
export function logConfirmed(action: string, detail: string) {
  return $fetch('/api/ai/activity', { method: 'POST', body: { action, detail } }).catch(() => {})
}

export function aiHealth() {
  return $fetch<{ ok: boolean; error?: string }>('/api/ai/health')
}

export interface AiSettings {
  installed: boolean
  enabled: boolean
  provider: AiProvider
  url: string
  effective_url: string
  model: string
  has_key: boolean
  embed_url: string
  embed_model: string
  index_mail: boolean
  local: boolean
  read_mail: boolean
  read_agenda: boolean
  read_people: boolean
  read_notes: boolean
  read_tasks: boolean
  /** The Notes / Tasks apps are in the dock (their switches show only then). */
  notes_app: boolean
  tasks_app: boolean
  brief_enabled: boolean
  brief_time: string
  mcp_enabled: boolean
}

export type AiSettingsUpdate = Partial<
  Omit<AiSettings, 'has_key' | 'effective_url' | 'local' | 'installed' | 'enabled' | 'notes_app' | 'tasks_app'>
> & {
  api_key?: string
}

export const AI_PROVIDERS: { value: AiProvider; label: string; hint: string; url: string }[] = [
  { value: 'ollama', label: 'Ollama (local)', hint: 'Runs on your computer or server. Nothing leaves your network.', url: 'http://localhost:11434' },
  {
    value: 'openai',
    label: 'OpenAI-compatible',
    hint: 'OpenAI, Mistral, Groq, OpenRouter, LM Studio, vLLM… Address and API key from the provider.',
    url: 'https://api.openai.com/v1',
  },
  { value: 'anthropic', label: 'Anthropic (Claude)', hint: 'API key from console.anthropic.com.', url: 'https://api.anthropic.com' },
  {
    value: 'claude-code',
    label: 'Claude Code (your Claude plan)',
    hint: 'Your Claude plan (Pro, Max, Team…) through Claude Code. No API key: install and sign in below.',
    url: 'claude',
  },
]

// ---- MCP: connecting Claude Code (or another AI app) to Havre ------------------------

export interface McpToken {
  id: number
  name: string
  created_at: number
  last_used_at: number | null
}

export function fetchMcpTokens() {
  return $fetch<McpToken[]>('/api/mcp/tokens')
}

export function createMcpToken(name: string) {
  return $fetch<{ token: McpToken; secret: string }>('/api/mcp/tokens', { method: 'POST', body: { name } })
}

export function revokeMcpToken(id: number) {
  return $fetch(`/api/mcp/tokens/${id}`, { method: 'DELETE' })
}

/** Shared by every page: is AI on, can we search by meaning? */
export function useAiStatus() {
  const status = useState<AiStatus>('ai-status', () => ({ enabled: false, search: false }))
  const loaded = useState('ai-status-loaded', () => false)
  async function refresh() {
    status.value = await $fetch<AiStatus>('/api/ai/status').catch(() => ({ enabled: false, search: false }))
    loaded.value = true
  }
  if (!loaded.value && import.meta.client) refresh()
  return { status, refresh }
}

export function fetchAiSettings() {
  return $fetch<AiSettings>('/api/ai/settings')
}

export async function saveAiSettings(update: AiSettingsUpdate) {
  const saved = await $fetch<AiSettings>('/api/ai/settings', { method: 'PATCH', body: update })
  await useAiStatus().refresh()
  return saved
}

export function fetchAiModels() {
  return $fetch<string[]>('/api/ai/models')
}

export function testAi() {
  return $fetch<{ reply: string }>('/api/ai/test', { method: 'POST' })
}

/** The browser's distance from UTC (minutes, as `getTimezoneOffset`). */
export const utcOffset = () => new Date().getTimezoneOffset()

/**
 * Calls a streaming route: `onText` gets the whole text so far each time a
 * piece arrives. Rejects with the server's message if it fails at once.
 */
export async function streamAi(
  path: string,
  body: unknown,
  onText: (text: string) => void,
  signal?: AbortSignal,
): Promise<string> {
  const response = await fetch(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    credentials: 'same-origin',
    signal,
  })
  if (!response.ok) {
    const data = await response.json().catch(() => null)
    if (response.status === 401) await navigateTo('/login')
    throw new Error(data?.error ?? `${response.status} ${response.statusText}`)
  }
  const reader = response.body!.getReader()
  const decoder = new TextDecoder()
  let text = ''
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    text += decoder.decode(value, { stream: true })
    onText(text)
  }
  text += decoder.decode()
  onText(text)
  return text
}

/** A text that ends with an error line (`⚠ …`), split in two. */
export function splitWarning(text: string): { text: string; warning: string | null } {
  const at = text.lastIndexOf('\n\n⚠ ')
  return at < 0 ? { text, warning: null } : { text: text.slice(0, at), warning: text.slice(at + 4) }
}

// ---- Search by meaning ------------------------------------------------------------

export interface AiHit {
  account: number
  folder: string
  uid: number
  subject: string
  from: Person | null
  date: string | null
  score: number
}

export function searchByMeaning(q: string, limit = 20) {
  return $fetch<AiHit[]>('/api/ai/search', { query: { q, limit } })
}

// ---- The assistant ------------------------------------------------------------------

export interface AiTurn {
  role: 'user' | 'assistant'
  text: string
}

export type AiProposal =
  | { kind: 'event'; title: string; start: number; end: number; location: string; guests: string[]; notes: string }
  | { kind: 'email'; to: string; subject: string; body: string }
  | { kind: 'move'; event: number; title: string; from_start: number; from_end: number; start: number; end: number }
  | {
      kind: 'task'
      title: string
      due: number | null
      due_at: number | null
      notes: string
      /** Where it goes: "Havre · Inbox". */
      list: string
      person: PersonLink | null
      event: EventLink | null
      /** Made from a note's `- [ ]` line. */
      from_note: { note: number; line: number; title: string } | null
    }
  | { kind: 'note'; title: string; body: string; people: PersonLink[]; event: EventLink | null }

export interface AiAnswer {
  text: string
  steps: { tool: string; label: string }[]
  proposals: AiProposal[]
  messages: Record<string, AiHit>
  events: Record<string, AgendaEvent>
  /** Missing in answers kept from before notes and tasks. */
  notes?: Record<string, NoteSummary>
  tasks?: Record<string, Task>
}

export function askAi(turns: AiTurn[]) {
  return $fetch<AiAnswer>('/api/ai/ask', { method: 'POST', body: { turns, utc_offset: utcOffset() } })
}

// ---- Conversations (the AI app; Ctrl+K chats are kept there too) ------------------

export interface AiConversationSummary {
  id: number
  title: string
  updated_at: number
  /** The start of the last answer. */
  last: string
}

export interface AiStoredTurn {
  id: number
  role: 'user' | 'assistant'
  text: string
  answer: AiAnswer | null
  /** Proposals already confirmed (positions in `answer.proposals`). */
  done: number[]
  created_at: number
}

export interface AiConversation extends AiConversationSummary {
  turns: AiStoredTurn[]
}

export function fetchConversations() {
  return $fetch<AiConversationSummary[]>('/api/ai/conversations')
}
export function fetchConversation(id: number) {
  return $fetch<AiConversation>(`/api/ai/conversations/${id}`)
}
/** Asks in a conversation (none: starts one); returns it, with the new answer. */
export function askInConversation(conversation: number | null, question: string) {
  return $fetch<AiConversation>('/api/ai/conversations/ask', {
    method: 'POST',
    body: { conversation, question, utc_offset: utcOffset() },
  })
}
export function renameConversation(id: number, title: string) {
  return $fetch<AiConversation>(`/api/ai/conversations/${id}`, { method: 'PATCH', body: { title } })
}
export function deleteConversation(id: number) {
  return $fetch(`/api/ai/conversations/${id}`, { method: 'DELETE' })
}
/** Remembers that you confirmed proposal `proposal` of turn `turn`. */
export function markProposalDone(conversation: number, turn: number, proposal: number) {
  return $fetch(`/api/ai/conversations/${conversation}/turns/${turn}/done`, {
    method: 'PUT',
    body: { proposal },
  }).catch(() => {})
}

// ---- Claude Code: installed? signed in? ------------------------------------------------

export interface ClaudeStatus {
  /** Where `claude` was found; null: not installed. */
  command: string | null
  version: string | null
  logged_in: boolean
  auth_method: string | null
}

export function fetchClaudeStatus() {
  return $fetch<ClaudeStatus>('/api/ai/claude-code')
}

/** Starts `claude auth login` on the server; returns the address to open. */
export function startClaudeLogin() {
  return $fetch<{ url: string }>('/api/ai/claude-code/login', { method: 'POST' })
}

export function completeClaudeLogin(code: string) {
  return $fetch<{ success: boolean; output: string }>('/api/ai/claude-code/login/complete', {
    method: 'POST',
    body: { code },
  })
}

export function cancelClaudeLogin() {
  return $fetch('/api/ai/claude-code/login', { method: 'DELETE' })
}

export function claudeLogout() {
  return $fetch('/api/ai/claude-code/logout', { method: 'POST' })
}
