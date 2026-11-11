<template>
    <div class="user-picker">
        <!-- The title page, like the theme's book covers: its emblem (a key,
             a window on the stars, an arch, a moon), then the wordmark in its
             metal between two of its ornaments. -->
        <header class="picker-hero">
            <span class="picker-emblem" aria-hidden="true"><span class="ornament -art"></span></span>
            <span class="ornament -rule picker-rule" aria-hidden="true"></span>
            <h1 class="picker-logo display-title foil-text">Librarium</h1>
            <p class="picker-tagline">{{ $t('user.tagline') }}</p>
            <span class="ornament -rule picker-rule -bottom" aria-hidden="true"></span>
        </header>

        <div class="picker-content">
            <!-- The users can't be listed: the library folder (often a NAS
                 share) isn't there, Librarium may not read it (macOS privacy
                 settings), or reading it failed. Say why, offer Retry — never
                 offer to create users in a folder that can't be read. -->
            <div v-if="usersStore.unavailable" class="picker-unavailable" role="alert">
                <vue-feather :type="unavailableIcon" size="32" />
                <h2>{{ unavailableTitle }}</h2>
                <p>{{ unavailableBody }}</p>
                <button class="button is-primary" :disabled="usersStore.loading" @click="retry">
                    {{ $t('library.retry') }}
                </button>
            </div>

            <h2 v-if="!usersStore.unavailable && users.length > 0" class="picker-question">{{ $t('user.whoIsReading') }}</h2>
            <div v-if="!usersStore.unavailable" class="user-grid">
                <button
                    v-for="user in users"
                    :key="user"
                    class="user-card"
                    @click="select(user)"
                >
                    <!-- The user's own colour, the same as in the sidebar. -->
                    <div class="user-card_avatar" :style="{ background: avatarColor(user) }" aria-hidden="true">
                        {{ initials(user) }}
                    </div>
                    <span class="user-card_name">{{ user }}</span>
                </button>

                <button v-if="!showForm" class="user-card user-card--add" @click="showForm = true">
                    <div class="user-card_avatar user-card_avatar--add">
                        <vue-feather type="plus" size="24" />
                    </div>
                    <span class="user-card_name">{{ $t('user.add') }}</span>
                </button>
            </div>

            <Transition name="slide-up">
                <form v-if="showForm && !usersStore.unavailable" class="add-form" @submit.prevent="submitCreate">
                    <input
                        ref="nameInput"
                        v-model="newName"
                        class="input add-form_input"
                        :placeholder="$t('user.namePh')"
                        maxlength="40"
                        autocomplete="off"
                    />
                    <div class="add-form_actions">
                        <button type="button" class="button is-ghost" @click="cancelForm">{{ $t('user.cancel') }}</button>
                        <button type="submit" class="button is-primary" :disabled="!newName.trim() || creating">
                            {{ creating ? $t('user.creating') : $t('user.create') }}
                        </button>
                    </div>
                    <p v-if="error" class="add-form_error">{{ error }}</p>
                </form>
            </Transition>
        </div>

        <div class="picker-lang" role="group" :aria-label="$t('sidebar.languageLabel')">
            <button
                type="button"
                lang="fr"
                aria-label="Français"
                :aria-pressed="currentLang === 'fr' ? 'true' : 'false'"
                :class="{ active: currentLang === 'fr' }"
                @click="setLang('fr')"
            >
                FR
            </button>
            <button
                type="button"
                lang="en"
                aria-label="English"
                :aria-pressed="currentLang === 'en' ? 'true' : 'false'"
                :class="{ active: currentLang === 'en' }"
                @click="setLang('en')"
            >
                EN
            </button>
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent, nextTick } from 'vue'
    import { mapStores } from 'pinia'
    import { useUsersStore } from '@/stores/users'
    import { currentLang, setLang } from '@/i18n'
    import { LIBRARY_UNAVAILABLE, PERMISSION_DENIED } from '@/utils/errors'
    import { initials, avatarColor } from '@/utils/avatar'

    export default defineComponent({
        name: 'UserPicker',

        data() {
            return {
                showForm: false,
                newName: '',
                creating: false,
                error: null as string | null,
            }
        },

        computed: {
            ...mapStores(useUsersStore),

            users(): string[] {
                return this.usersStore.users
            },

            /** Why the users can't be listed, as the store's error code says. */
            unavailableKind(): 'missing' | 'denied' | 'failed' {
                const code = this.usersStore.errorCode
                if (code === PERMISSION_DENIED) { return 'denied' }
                // No code: an older backend's "unreachable" (or no error at all).
                if (code === LIBRARY_UNAVAILABLE || code === null) { return 'missing' }
                return 'failed'
            },

            unavailableIcon(): string {
                return { missing: 'hard-drive', denied: 'lock', failed: 'alert-triangle' }[this.unavailableKind]
            },

            unavailableTitle(): string {
                return {
                    missing: this.$t('library.unavailableTitle'),
                    denied: this.$t('library.permissionDeniedTitle'),
                    failed: this.$t('user.listFailedTitle'),
                }[this.unavailableKind]
            },

            /** The backend's message says where to fix a permission problem (System Settings…). */
            unavailableBody(): string {
                if (this.unavailableKind === 'missing') {
                    return this.$t('library.unavailableBody', { path: this.usersStore.libraryRoot ?? '' })
                }
                return this.usersStore.error ?? ''
            },

            currentLang(): string {
                return currentLang.value
            },
        },

        watch: {
            async showForm(v: boolean) {
                if (v) {
                    await nextTick()
                    ;(this.$refs.nameInput as HTMLInputElement | undefined)?.focus()
                }
            },
        },

        methods: {
            initials,
            avatarColor,
            setLang,

            select(name: string) {
                void this.usersStore.selectUser(name)
            },

            /** "Library not reachable" → Retry: read the users again. */
            async retry() {
                await this.usersStore.retry()
            },

            async submitCreate() {
                const name = this.newName.trim()
                if (!name) { return }
                this.creating = true
                this.error = null
                const result = await this.usersStore.createUser(name)
                this.creating = false
                if (result) {
                    void this.usersStore.selectUser(result)
                    this.cancelForm()
                } else {
                    this.error = this.usersStore.error ?? this.$t('user.createError')
                }
            },

            cancelForm() {
                this.showForm = false
                this.newName = ''
                this.error = null
            },
        },
    })
</script>

<style lang="scss" scoped>
.user-picker {
    position: fixed;
    inset: 0;
    background: transparent;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-6);
    padding: var(--space-6);
    overflow-y: auto;
    z-index: 300;
}

// ── The title page ───────────────────────────────────────────────
.picker-hero {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    animation: picker-rise 900ms var(--ease-out) both;
}

// The theme's cover emblem, in its metal, with a halo of its light (the
// glow is on the wrapper: a filter on the masked shape itself is clipped).
.picker-emblem {
    display: block;
    margin-bottom: 8px;
    filter: drop-shadow(0 0 22px color-mix(in srgb, var(--accent) 45%, transparent));
    animation: picker-glow 7s ease-in-out infinite alternate;

    .ornament { width: 104px; }
}

@keyframes picker-glow {
    from { filter: drop-shadow(0 0 14px color-mix(in srgb, var(--accent) 30%, transparent)); }
    to   { filter: drop-shadow(0 0 28px color-mix(in srgb, var(--accent) 55%, transparent)); }
}

.picker-rule {
    width: 260px;

    &.-bottom { transform: scaleY(-1); }
}

.picker-logo {
    font-size: 68px;
    line-height: 1;
    padding-left: var(--display-tracking);
    filter: drop-shadow(0 4px 22px color-mix(in srgb, var(--accent) 26%, transparent));
}

.picker-tagline {
    font-family: var(--font-serif);
    font-style: italic;
    font-size: calc(var(--font-size-lg) * var(--serif-scale));
    font-weight: 500;
    letter-spacing: 0.02em;
    color: var(--text-secondary);
}

@keyframes picker-rise {
    from { opacity: 0; transform: translateY(14px); }
    to   { opacity: 1; transform: none; }
}

// ── The users ─────────────────────────────────────────────────────
.picker-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-5);
    padding: var(--space-4) var(--space-6) var(--space-6);
    animation: picker-rise 900ms 120ms var(--ease-out) both;
}

.picker-question {
    font-family: var(--font-heading);
    font-weight: var(--heading-weight);
    letter-spacing: var(--heading-tracking);
    font-size: var(--font-size-xl);
    color: var(--text-primary);
}

.picker-unavailable {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    max-width: 480px;
    text-align: center;
    color: var(--text-secondary);

    h2 {
        font-family: var(--font-heading);
        font-weight: var(--heading-weight);
        font-size: var(--font-size-xl);
        color: var(--text-primary);
    }

    p {
        word-break: break-word;
    }
}

.user-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 18px;
    justify-content: center;
    // Room for a family on one row, "Add user" included.
    max-width: min(1160px, calc(100vw - 2 * var(--space-6)));
}

// A medallion: the user's colour in a gilt ring, on a pane of glass.
.user-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 24px 18px 20px;
    width: 148px;
    background: color-mix(in srgb, var(--glass) 100%, transparent);
    backdrop-filter: blur(14px) saturate(130%);
    -webkit-backdrop-filter: blur(14px) saturate(130%);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    cursor: pointer;
    transition:
        border-color var(--transition),
        transform var(--duration-slow) var(--ease-lift),
        box-shadow var(--duration-slow) var(--ease-out);

    &:hover {
        border-color: var(--hairline);
        transform: translateY(-5px);
        box-shadow: var(--glow-card);

        .user-card_avatar { box-shadow: 0 0 0 3px var(--bg-elevated), 0 0 0 4.5px var(--accent), 0 0 28px color-mix(in srgb, var(--accent) 45%, transparent); }
    }

    &--add {
        border-style: dashed;
    }

    &_avatar {
        width: 68px;
        height: 68px;
        border-radius: 50%;
        display: flex;
        align-items: center;
        justify-content: center;
        // The user's cover cloth (utils/avatar.ts), lettered like a cover.
        font-family: var(--font-heading);
        font-size: 26px;
        font-weight: var(--heading-weight);
        color: var(--cover-ink);
        text-shadow: 0 1px 2px rgba(0,0,0,.35);
        flex-shrink: 0;
        box-shadow: 0 0 0 3px var(--bg-elevated), 0 0 0 4.5px color-mix(in srgb, var(--accent) 75%, transparent);
        transition: box-shadow var(--duration-slow) var(--ease-out);

        &--add {
            background: transparent;
            color: var(--accent-text);
            border: 1.5px dashed var(--hairline);
            box-shadow: none;
        }
    }

    &_name {
        font-family: var(--font-heading);
        font-weight: var(--heading-weight);
        letter-spacing: var(--heading-tracking);
        font-size: var(--font-size-md);
        color: var(--text-primary);
        text-align: center;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 100%;
    }
}

.add-form {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    width: 320px;

    &_input {
        font-size: 16px;
        padding: 10px 16px;
        text-align: center;
        border-radius: var(--radius-full);
    }

    &_actions {
        display: flex;
        gap: 10px;
    }

    &_error {
        font-size: 12px;
        color: var(--danger);
    }
}

.picker-lang {
    position: absolute;
    bottom: 16px;
    right: 20px;
    display: flex;
    align-items: center;
    background: color-mix(in srgb, var(--text-secondary) 8%, transparent);
    border: 1px solid var(--border);
    border-radius: 99px;
    padding: 2px;
    gap: 1px;

    button {
        background: none;
        border: none;
        cursor: pointer;
        font-size: 12px;
        font-weight: 600;
        letter-spacing: 0.04em;
        color: var(--text-muted);
        padding: 3px 10px;
        border-radius: 99px;
        transition: background var(--transition), color var(--transition);

        &.active {
            background: var(--accent-strong);
            color: var(--on-accent);
        }

        &:not(.active):hover {
            color: var(--text-primary);
        }
    }
}

@media (prefers-reduced-motion: reduce) {
    .picker-hero, .picker-content, .picker-emblem { animation: none; }
    .user-card:hover { transform: none; }
}

@media (max-height: 760px) {
    .picker-logo { font-size: 52px; }
    .picker-emblem .ornament { width: 76px; }
}
</style>
