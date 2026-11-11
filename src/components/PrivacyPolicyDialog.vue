<template>
    <BaseModal
        size="lg"
        :title="$t('privacyPolicy.title')"
        close-on-backdrop
        @close="$emit('close')"
    >
        <div ref="content" class="privacy-policy">
            <p v-if="lang !== 'en'" class="privacy-policy__note">{{ $t('privacyPolicy.englishOnly') }}</p>
            <!--
                PRIVACY.md, bundled at build time and rendered from a parsed
                tree (utils/safeText.ts parseSafeMarkdown), never with v-html.
            -->
            <template v-for="(block, bi) in blocks" :key="bi">
                <component
                    :is="headingTag(block.level)"
                    v-if="block.type === 'heading'"
                    :data-anchor="block.id"
                    tabindex="-1"
                    class="privacy-policy__heading"
                >
                    <MarkdownInlines :nodes="block.inlines" @anchor="goTo" />
                </component>
                <p v-else-if="block.type === 'paragraph'">
                    <MarkdownInlines :nodes="block.inlines" @anchor="goTo" />
                </p>
                <ul v-else-if="block.type === 'list'">
                    <li v-for="(item, ii) in block.items" :key="ii">
                        <MarkdownInlines :nodes="item" @anchor="goTo" />
                    </li>
                </ul>
                <table v-else-if="block.type === 'table'">
                    <thead>
                        <tr>
                            <th v-for="(cell, ci) in block.header" :key="ci" scope="col">
                                <MarkdownInlines :nodes="cell" @anchor="goTo" />
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr v-for="(row, ri) in block.rows" :key="ri">
                            <td v-for="(cell, ci) in row" :key="ci">
                                <MarkdownInlines :nodes="cell" @anchor="goTo" />
                            </td>
                        </tr>
                    </tbody>
                </table>
            </template>
        </div>

        <template #footer>
            <BaseButton variant="primary" data-autofocus @click="$emit('close')">{{ $t('common.close') }}</BaseButton>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import MarkdownInlines from '@/components/MarkdownInlines.vue'
    import { parseSafeMarkdown, type MarkdownBlock } from '@/utils/safeText'
    import { currentLang, type Lang } from '@/i18n'
    import privacyPolicy from '../../PRIVACY.md?raw'

    /**
     * The privacy policy (PRIVACY.md at the root of the repository), shown in
     * the app: from the consent dialog and from Settings › Privacy &
     * analytics. The file is imported as text at build time, so the app
     * always shows the policy of its own version, offline.
     *
     * It is rendered from a parsed tree (headings, paragraphs, lists,
     * tables, emphasis, code, links), never as HTML; HTML comments in the
     * file (notes for the maintainer) are left out. The document's own title
     * is the dialog's title. The policy exists in English only.
     */
    export default defineComponent({
        name: 'PrivacyPolicyDialog',

        components: { BaseModal, BaseButton, MarkdownInlines },

        props: {
            /** The Markdown to show; PRIVACY.md unless a test passes another. */
            source: { type: String, default: privacyPolicy },
        },

        emits: ['close'],

        computed: {
            /** Without the level-1 heading: the dialog's title says it. */
            blocks(): MarkdownBlock[] {
                return parseSafeMarkdown(this.source).filter(b => !(b.type === 'heading' && b.level === 1))
            },

            lang(): Lang {
                return currentLang.value
            },
        },

        methods: {
            /** The dialog's title is an h2: the document's sections are h3, then h4. */
            headingTag(level: number): string {
                return level <= 2 ? 'h3' : level === 3 ? 'h4' : 'h5'
            },

            /** A link to another section of the policy: scroll to it, and move the focus there. */
            goTo(id: string) {
                const content = this.$refs.content as HTMLElement | undefined
                const target = Array.from(content?.querySelectorAll<HTMLElement>('[data-anchor]') ?? [])
                    .find(el => el.dataset.anchor === id)
                if (!target) { return }
                target.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
                target.focus({ preventScroll: true })
            },
        },
    })
</script>

<style lang="scss" scoped>
.privacy-policy {
    font-size: var(--font-size-sm);
    line-height: var(--line-height);
    color: var(--text-secondary);
    overflow-wrap: anywhere;

    p, ul, table { margin: 0 0 var(--space-3); }

    ul { padding-left: 20px; }
    li + li { margin-top: var(--space-1); }

    strong { color: var(--text-primary); font-weight: var(--font-weight-semibold); }

    code {
        font-family: var(--font-mono);
        font-size: 0.92em;
        padding: 0 3px;
        border-radius: var(--radius-sm);
        background: color-mix(in srgb, var(--text-secondary) 10%, transparent);
        color: var(--text-primary);
    }

    a {
        color: var(--accent-text);
        text-decoration: underline;
    }

    table {
        width: 100%;
        border-collapse: collapse;
        font-size: var(--font-size-xs);
    }

    th, td {
        padding: var(--space-1) var(--space-2);
        border: 1px solid var(--border);
        text-align: left;
        vertical-align: top;
    }

    th {
        color: var(--text-primary);
        font-weight: var(--font-weight-semibold);
        background: color-mix(in srgb, var(--text-secondary) 6%, transparent);
    }
}

.privacy-policy__heading {
    margin: var(--space-5) 0 var(--space-2);
    color: var(--text-primary);
    font-weight: var(--font-weight-semibold);
    outline: none;
    scroll-margin-top: var(--space-3);

    &:is(h3) { font-size: var(--font-size-lg); }
    &:is(h4, h5) { font-size: var(--font-size-md); }
}

.privacy-policy__note {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--text-secondary) 6%, transparent);
    font-size: var(--font-size-xs);
}
</style>
