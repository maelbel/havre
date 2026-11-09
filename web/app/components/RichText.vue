<script setup lang="ts">
import { Extension, Node, mergeAttributes } from '@tiptap/core'
import type { Editor } from '@tiptap/core'

// Rich text for Mail: the compose window's text, and signatures. Nuxt UI's
// `UEditor` (TipTap) with only what Havre keeps when it cleans a message on
// the server (bold, italic, underline, strikethrough, links, lists, quotes):
// the editor can't produce what would be removed.
const props = withDefaults(
  defineProps<{
    /** Clean HTML. */
    modelValue: string
    placeholder?: string
    /** A signature: bold, italic and links only. */
    small?: boolean
    label?: string
  }>(),
  { placeholder: '', small: false, label: 'Message text' },
)
const emit = defineEmits<{ 'update:modelValue': [html: string] }>()

/**
 * The signature: a block of its own (`<div data-signature>`), so the server
 * finds it again to swap it when "From" changes. You can still edit inside.
 */
const Signature = Node.create({
  name: 'signature',
  group: 'block',
  content: 'block+',
  defining: true,
  parseHTML: () => [{ tag: 'div[data-signature]' }],
  renderHTML: ({ HTMLAttributes }) => ['div', mergeAttributes(HTMLAttributes, { 'data-signature': '' }), 0],
})

// ---- Links ------------------------------------------------------------------------
const linking = ref(false)
const linkUrl = ref('')
const linkError = ref<string | null>(null)
/** The cursor was in a link: the dialog offers to remove it. */
const inLinkShown = ref(false)
/** Only web and mail addresses: what the server keeps. */
function normalize(url: string) {
  const u = url.trim()
  if (!u) return null
  if (/^(https?:|mailto:)/i.test(u)) return u
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(u)) return `mailto:${u}`
  if (/^[\w-]+(\.[\w-]+)+(\/\S*)?$/.test(u)) return `https://${u}`
  return null
}
// The selection when the dialog opened: closing it puts the focus back in the
// text, which moves the selection, so the link goes where this says.
let range = { from: 0, to: 0 }
let inLink = false
/**
 * The selection as the browser has it. The editor learns of a selection
 * made with the keyboard a moment later, so a quick `Shift+←` then `Ctrl+K`
 * would otherwise link only part of it.
 */
function selected(e: Editor) {
  const s = window.getSelection()
  if (!s?.anchorNode || !s.focusNode || !e.view.dom.contains(s.anchorNode)) return e.state.selection
  try {
    const a = e.view.posAtDOM(s.anchorNode, s.anchorOffset)
    const b = e.view.posAtDOM(s.focusNode, s.focusOffset)
    return { from: Math.min(a, b), to: Math.max(a, b) }
  } catch {
    return e.state.selection
  }
}
function openLink() {
  const e = editor.value
  if (!e) return
  const { from, to } = selected(e)
  range = { from, to }
  e.commands.setTextSelection(range)
  inLink = e.isActive('link')
  inLinkShown.value = inLink
  linkUrl.value = (e.getAttributes('link').href as string | undefined) ?? ''
  linkError.value = null
  linking.value = true
}
function saveLink() {
  const e = editor.value
  if (!e) return
  const href = normalize(linkUrl.value)
  if (!href) {
    linkError.value = 'A web address (https://…) or an email address.'
    return
  }
  const chain = e.chain().focus().setTextSelection(range)
  if (range.from === range.to && !inLink) {
    // Nothing selected: the address becomes the text.
    chain
      .insertContent({
        type: 'text',
        text: linkUrl.value.trim(),
        marks: [{ type: 'link', attrs: { href } }],
      })
      .run()
  } else {
    // The selection, or the whole link the cursor is in; then the cursor goes
    // after it (what you type there isn't part of the link: `autolink` is off).
    chain.extendMarkRange('link').setLink({ href }).run()
    e.commands.setTextSelection(e.state.selection.to)
  }
  linking.value = false
}
function removeLink() {
  editor.value?.chain().focus().setTextSelection(range).extendMarkRange('link').unsetLink().run()
  linking.value = false
}

/** `Ctrl/⌘ + K`: a link (bold, italic, underline come with TipTap). */
const Shortcuts = Extension.create({
  name: 'havreShortcuts',
  addKeyboardShortcuts: () => ({
    'Mod-k': () => {
      openLink()
      return true
    },
  }),
})

// TipTap's starter kit, without what a message can't keep.
const starterKit = {
  heading: false as const,
  codeBlock: false as const,
  horizontalRule: false as const,
  code: false as const,
  link: {
    openOnClick: false,
    // Off: with it on, TipTap makes links grow with what you type right after
    // them. A pasted address still becomes a link (`linkOnPaste`).
    autolink: false,
    defaultProtocol: 'https',
    isAllowedUri: (url: string) => /^(https?:|mailto:)/i.test(url),
  },
}
const extensions = [Signature, Shortcuts]

const items = computed(() => {
  const mark = (m: string, icon: string, label: string) => ({
    kind: 'mark',
    mark: m,
    icon: `i-havre-${icon}`,
    'aria-label': label,
    tooltip: { text: label },
  })
  const link = {
    icon: 'i-havre-link',
    'aria-label': 'Link',
    tooltip: { text: 'Link' },
    active: !!editor.value?.isActive('link'),
    onClick: openLink,
  }
  if (props.small) return [[mark('bold', 'bold', 'Bold'), mark('italic', 'italic', 'Italic'), link]]
  return [
    [
      mark('bold', 'bold', 'Bold'),
      mark('italic', 'italic', 'Italic'),
      mark('underline', 'underline', 'Underline'),
      mark('strike', 'strike', 'Strikethrough'),
    ],
    [link],
    [
      {
        kind: 'bulletList',
        icon: 'i-havre-list-bullet',
        'aria-label': 'Bulleted list',
        tooltip: { text: 'Bulleted list' },
      },
      {
        kind: 'orderedList',
        icon: 'i-havre-list-ordered',
        'aria-label': 'Numbered list',
        tooltip: { text: 'Numbered list' },
      },
      {
        kind: 'blockquote',
        icon: 'i-havre-quote',
        'aria-label': 'Quote',
        tooltip: { text: 'Quote' },
      },
    ],
    [
      {
        kind: 'clearFormatting',
        icon: 'i-havre-clear-format',
        'aria-label': 'Clear formatting',
        tooltip: { text: 'Clear formatting' },
      },
    ],
  ]
})

// The TipTap editor, for the parent (focus, the AI writing in it). A shallow
// ref: Vue doesn't need to watch inside the editor (and `ref` would change
// its type into a "reactive copy" that isn't an `Editor` any more).
const root = shallowRef<{ editor?: Editor } | null>(null)
const editor = computed(() => root.value?.editor ?? null)
defineExpose({ editor })

const html = computed({
  get: () => props.modelValue,
  set: (value: string) => emit('update:modelValue', value),
})
</script>

<template>
  <!-- One element at the top, so a parent's `class` and scoped styles reach it
       (the link dialog is drawn elsewhere on the page anyway). -->
  <div class="rich-text" :class="{ small }">
    <UEditor
      ref="root"
      v-slot="{ editor: e }"
      v-model="html"
      content-type="html"
      :starter-kit="starterKit"
      :image="false"
      :mention="false"
      :extensions="extensions"
      :placeholder="{ placeholder, mode: 'firstLine', showOnlyCurrent: false }"
      :editor-props="{
        attributes: {
          'aria-label': label,
          'aria-multiline': 'true',
          role: 'textbox',
        },
      }"
      class="editor"
    >
      <UEditorToolbar :editor="e" :items="items" class="toolbar" />
    </UEditor>
    <UModal v-model:open="linking" title="Link" description="A web page or an email address.">
      <template #body>
        <form id="rich-link" @submit.prevent="saveLink">
          <UFormField label="Address" :error="linkError ?? undefined">
            <UInput v-model="linkUrl" placeholder="https://… or name@example.com" autofocus class="w-full" />
          </UFormField>
        </form>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton v-if="inLinkShown" color="neutral" variant="ghost" class="mr-auto" @click="removeLink">
            Remove the link
          </UButton>
          <UButton color="neutral" variant="outline" @click="linking = false">Cancel</UButton>
          <UButton type="submit" form="rich-link">Save</UButton>
        </div>
      </template>
    </UModal>
  </div>
</template>

<style scoped>
.rich-text,
.editor {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.editor {
  flex: 1;
}
/* Only the text scrolls: the toolbar stays at the top. */
.editor :deep([data-slot='content']) {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
/* The toolbar: one row, scrolling sideways on a phone. */
.toolbar {
  flex: none;
  overflow-x: auto;
  scrollbar-width: none;
  border-bottom: 1px solid var(--line);
  padding: 4px 10px;
}
.rich-text :deep(.tiptap) {
  outline: none;
  min-height: 120px;
  padding: 14px 16px;
  font-size: 14.5px;
  line-height: 1.6;
  color: var(--text);
  /* "--" stays two dashes (the signature line). */
  font-variant-ligatures: none;
}
.rich-text.small :deep(.tiptap) {
  min-height: 70px;
  padding: 10px 12px;
}
.rich-text :deep(.tiptap p) {
  margin: 0 0 0.5em;
}
.rich-text :deep(.tiptap ul) {
  list-style: disc;
  padding-left: 1.4em;
  margin: 0 0 0.5em;
}
.rich-text :deep(.tiptap ol) {
  list-style: decimal;
  padding-left: 1.4em;
  margin: 0 0 0.5em;
}
.rich-text :deep(.tiptap li p) {
  margin: 0;
}
.rich-text :deep(.tiptap blockquote) {
  margin: 0 0 0.5em 2px;
  padding-left: 12px;
  border-left: 2px solid var(--line2);
  color: var(--muted);
}
.rich-text :deep(.tiptap a) {
  color: var(--accent);
  text-decoration: underline;
}
.rich-text :deep(.tiptap div[data-signature]) {
  color: var(--muted);
  font-size: 13.5px;
}
/* The placeholder: on the first line while it's empty. (Nuxt UI's "first
   line" mode waits for the whole text to be empty, which never happens with
   a signature below, and its other mode puts it on every empty line.) */
.rich-text :deep(.tiptap > p.is-empty:first-child::before) {
  content: attr(data-placeholder);
  color: var(--faint);
  float: left;
  height: 0;
  pointer-events: none;
}
</style>
