<script setup lang="ts">
import type { RemoteKind, RemoteSettings, RemoteStatus } from '~/composables/usePlatform'

// Settings › Backups › Off-site copy: where sealed copies of the backups go
// (a WebDAV folder or an S3 bucket), and how the last one went.
//
// The password / secret key is write-only: the page sends it when typed and
// only learns afterwards that one is saved (`has_secret`).
const props = defineProps<{ status: RemoteStatus }>()
const emit = defineEmits<{ changed: [] }>()
const toast = useToasts()

// The form keeps the secret as a plain string (empty = keep the saved one).
const form = reactive<RemoteSettings & { secret: string }>({ ...props.status.settings, secret: '' })
watch(
  () => props.status.settings,
  (s) => Object.assign(form, s, { secret: '' }),
)
const busy = ref<'test' | 'save' | 'forget' | null>(null)
const tested = ref<{ ok: boolean; text: string } | null>(null)
const confirmForget = ref(false)
// A test result is about the destination it tried.
watch(
  () => form.kind,
  () => (tested.value = null),
)

const kinds = [
  { label: 'WebDAV folder', value: 'webdav', description: 'Nextcloud, a NAS, any WebDAV server.' },
  { label: 'S3 bucket', value: 's3', description: 'Backblaze B2, MinIO, Scaleway, AWS…' },
]
const s3 = computed(() => form.kind === 's3')
const secretLabel = computed(() => (s3.value ? 'Secret key' : 'Password'))
const secretHint = computed(() =>
  props.status.settings.has_secret ? 'Saved — type a new one to change it' : s3.value ? '' : 'For Nextcloud, an app password',
)

function body(): RemoteSettings {
  return { ...form, keep: Number(form.keep), secret: form.secret.trim() || null }
}

async function test() {
  busy.value = 'test'
  tested.value = null
  try {
    const { message } = await testRemote(body())
    tested.value = { ok: true, text: message }
  } catch (err) {
    tested.value = { ok: false, text: apiErrorMessage(err) }
  } finally {
    busy.value = null
  }
}

/** Saves the form; `enabled` turns copies on or off (checked by the server first). */
async function save(enabled = form.enabled) {
  busy.value = 'save'
  try {
    await saveRemote({ ...body(), enabled })
    form.secret = ''
    toast.show(enabled ? 'Off-site copies are on' : 'Saved')
    emit('changed')
  } catch (err) {
    form.enabled = props.status.settings.enabled
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

async function forget() {
  if (!confirmForget.value) {
    confirmForget.value = true
    return
  }
  confirmForget.value = false
  busy.value = 'forget'
  try {
    await forgetRemote()
    tested.value = null
    emit('changed')
  } catch (err) {
    toast.show(apiErrorMessage(err), { tone: 'error' })
  } finally {
    busy.value = null
  }
}

/** The line at the top: how the last copy went. */
const line = computed(() => {
  const s = props.status
  if (!s.settings.enabled) return { tone: 'off', text: 'Off: backups stay on this server only.' }
  if (s.last_error) {
    const retry = s.next_try_at ? ` Next try ${whenShort(s.next_try_at)}.` : ''
    return { tone: 'bad', text: `The last copy failed: ${s.last_error}${retry}` }
  }
  if (s.pending) return { tone: 'warn', text: 'A backup is waiting to be copied (within a minute).' }
  if (!s.last_at) return { tone: 'warn', text: 'On. The next backup will be copied.' }
  return { tone: 'ok', text: `Last copy ${whenShort(s.last_at)}${s.last_size ? ` · ${bytes(s.last_size)}` : ''}` }
})
</script>

<template>
  <section class="block offsite" aria-labelledby="offsite-title">
    <div id="offsite-title" class="eyebrow">Off-site copy</div>
    <p class="note">
      After each backup, Havre encrypts it and sends it to another place, so a dead disk or a lost server isn't the
      end of your data. The copies are sealed with your <strong>recovery key</strong>: the place that stores them can't
      read them — and without the key, nobody can, you included.
    </p>

    <div class="state" :class="line.tone" aria-live="polite">
      <span class="dot" aria-hidden="true" />
      <span class="grow">{{ line.text }}</span>
      <USwitch
        :model-value="form.enabled"
        :disabled="!!busy"
        label="Copy each backup"
        @update:model-value="(on) => save(on)"
      />
    </div>

    <URadioGroup
      v-model="form.kind"
      :items="kinds"
      variant="card"
      orientation="horizontal"
      aria-label="Where to send the copies"
      :ui="{ fieldset: 'gap-2.5', item: 'flex-1 p-3.5 has-data-[state=checked]:bg-(--raise)' }"
      @update:model-value="(k) => (form.kind = k as RemoteKind)"
    />

    <div class="fields">
      <UFormField :label="s3 ? 'Endpoint' : 'Folder address'" class="wide">
        <UInput
          v-model="form.url"
          :placeholder="s3 ? 'https://s3.eu-central-003.backblazeb2.com' : 'https://cloud.example.org/remote.php/dav/files/me/Havre'"
          autocomplete="off"
          spellcheck="false"
          class="w-full"
        />
      </UFormField>
      <template v-if="s3">
        <UFormField label="Bucket">
          <UInput v-model="form.bucket" placeholder="my-backups" autocomplete="off" class="w-full" />
        </UFormField>
        <UFormField label="Region">
          <UInput v-model="form.region" placeholder="eu-central-003" autocomplete="off" class="w-full" />
        </UFormField>
        <UFormField label="Folder in the bucket" help="Optional, e.g. havre/">
          <UInput v-model="form.prefix" placeholder="havre/" autocomplete="off" class="w-full" />
        </UFormField>
        <UFormField label="Addresses" help="Path-style for MinIO and Backblaze B2">
          <UCheckbox v-model="form.path_style" label="endpoint/bucket/file (path-style)" />
        </UFormField>
      </template>
      <UFormField :label="s3 ? 'Access key' : 'User name'">
        <UInput v-model="form.username" autocomplete="off" spellcheck="false" class="w-full" />
      </UFormField>
      <UFormField :label="secretLabel" :help="secretHint">
        <UInput
          v-model="form.secret"
          type="password"
          autocomplete="new-password"
          :placeholder="status.settings.has_secret ? '••••••••' : ''"
          class="w-full"
        />
      </UFormField>
      <UFormField label="Copies to keep there" help="Older ones Havre made are deleted; other files are never touched.">
        <UInputNumber v-model="form.keep" :min="1" :max="365" class="w-28" />
      </UFormField>
    </div>

    <p v-if="tested" class="tested" :class="tested.ok ? 'ok' : 'bad'" role="status">
      <HIcon :name="tested.ok ? 'check' : 'alert'" :size="14" />{{ tested.text }}
    </p>

    <div class="actions">
      <UButton color="neutral" variant="outline" type="button" :loading="busy === 'test'" :disabled="!!busy" @click="test">
        Test the connection
      </UButton>
      <UButton type="button" :loading="busy === 'save'" :disabled="!!busy" @click="save()">Save</UButton>
      <span class="grow" />
      <UButton
        v-if="status.settings.url || status.settings.has_secret"
        :color="confirmForget ? 'error' : 'neutral'"
        variant="ghost"
        type="button"
        :disabled="!!busy"
        @click="forget"
      >
        {{ confirmForget ? 'Forget it? The copies already sent stay there' : 'Forget this destination' }}
      </UButton>
    </div>

    <p class="note faint">
      To restore on a new machine: download the newest <code>.havre</code> file from there, then
      <code>havre restore &lt;file&gt; --key &lt;recovery key&gt;</code>.
    </p>
  </section>
</template>

<style scoped>
.offsite {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.state {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--raise);
  font-size: 13.5px;
}
.state .dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--faint);
  flex: none;
}
.state.ok .dot {
  background: var(--ok);
}
.state.warn .dot {
  background: var(--accent);
}
.state.bad .dot {
  background: var(--danger);
}
.state.bad {
  color: var(--danger);
}
.grow {
  flex: 1;
  min-width: 0;
}
.fields {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px 16px;
}
.fields .wide {
  grid-column: 1 / -1;
}
.tested {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  font-size: 13px;
}
.tested.ok {
  color: var(--ok);
}
.tested.bad {
  color: var(--danger);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.note {
  margin: 0;
  font-size: 13px;
  line-height: 1.55;
  color: var(--muted);
}
.faint {
  color: var(--faint);
}
code {
  font-family: var(--font-mono);
  font-size: 12px;
}
@media (max-width: 720px) {
  .fields {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
