<template>
    <BaseModal
        :width="480"
        close-on-backdrop
        @close="close"
    >
        <template #title>
            <vue-feather type="bell" size="16" class="messages-icon" aria-hidden="true" />
            {{ $t('messages.title') }}
        </template>

        <div class="messages-list">
            <article
                v-for="msg in messages"
                :key="msg.id"
                class="message-card"
            >
                <h3 class="message-card-title">{{ localized(msg).title }}</h3>
                <!--
                    Server text is rendered from a parsed tree, never with
                    v-html: Vue escapes every text node, so markup in a
                    message shows as text instead of running in the
                    privileged webview. See utils/safeText.ts.
                -->
                <div class="message-card-body">
                    <p v-for="(paragraph, pi) in render(localized(msg).content)" :key="pi">
                        <template v-for="(node, ni) in paragraph" :key="ni">
                            <br v-if="node.type === 'br'" />
                            <a
                                v-else-if="node.type === 'link'"
                                :href="node.href"
                                target="_blank"
                                rel="noopener noreferrer"
                            >{{ node.text }}</a>
                            <template v-else>{{ node.text }}</template>
                        </template>
                    </p>
                </div>
            </article>
        </div>

        <template #footer>
            <BaseButton variant="primary" data-autofocus @click="close">
                {{ $t('messages.gotIt') }}
            </BaseButton>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import type { PropType } from 'vue'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import type { ApiMessage } from '@/services/librarium-api'
    import { currentLang } from '@/i18n'
    import { parseSafeText, type SafeParagraph } from '@/utils/safeText'

    /**
     * News from the developer (the Librarium API's messages), shown once at
     * startup by App.vue. Closing it, however it is closed, marks them read.
     */
    export default defineComponent({
        name: 'MessagesModal',

        components: { BaseModal, BaseButton },

        props: {
            messages: {
                type: Array as PropType<ApiMessage[]>,
                required: true,
            },
        },

        emits: ['close'],

        methods: {
            render(content: string): SafeParagraph[] {
                return parseSafeText(content)
            },

            close() { this.$emit('close') },

            localized(msg: ApiMessage): { title: string; content: string } {
                const t = msg.translations
                return t[currentLang.value] ?? t['en'] ?? { title: '', content: '' }
            },
        },
    })
</script>

<style lang="scss" scoped>
.messages-icon {
    color: var(--accent-text);
    flex-shrink: 0;
}

.messages-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
}

.message-card {
    padding: var(--space-4);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
}

.message-card-title {
    margin: 0 0 6px;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-bold);
    color: var(--text-primary);
}

.message-card-body {
    margin: 0;
    font-size: var(--font-size-sm);
    line-height: 1.55;
    color: var(--text-secondary);

    :deep(strong) { color: var(--text-primary); }
    :deep(a) { color: var(--accent-text); }
    :deep(p) { margin: 0.4em 0; }
    :deep(p:first-child) { margin-top: 0; }
    :deep(p:last-child) { margin-bottom: 0; }
}
</style>
