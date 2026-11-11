import { defineStore } from 'pinia'

/** One open dialog. `id` is unique per open dialog (see `open`). */
export interface ModalEntry {
    id: string
    /** For debugging only: which component opened it. */
    owner: string
}

interface ModalsState {
    /** Open dialogs, bottom to top: the last one is the one the user sees. */
    stack: ModalEntry[]
}

let nextId = 0

/**
 * The stack of open dialogs.
 *
 * Every dialog is a BaseModal, which pushes itself when it opens and
 * removes itself when it closes. Two rules follow from the stack:
 *
 * - only the top dialog reacts to Escape and other keys (`isTop(id)`), so
 *   Escape in a confirmation closes the confirmation, not the dialog under it;
 * - while any dialog is open, global shortcuts are off (`hasOpenModal`,
 *   checked by the shortcuts store), so keys meant for a dialog never also
 *   act on the view behind it.
 *
 * Entries are removed by id, never by position, so a dialog closing out of
 * order (a parent unmounting a dialog that isn't on top) can't remove
 * another one.
 */
export const useModalsStore = defineStore('modals', {
    state: (): ModalsState => ({
        stack: [],
    }),

    getters: {
        hasOpenModal: (state): boolean => state.stack.length > 0,

        count: (state): number => state.stack.length,

        /** The id of the dialog on top, or null when none is open. */
        topId: (state): string | null => state.stack.length ? state.stack[state.stack.length - 1].id : null,

        /** Whether dialog `id` is the one on top (the only one that gets keys). */
        isTop(): (id: string) => boolean {
            return (id: string) => this.topId === id
        },
    },

    actions: {
        /** Registers a newly opened dialog on top of the stack; returns its id. */
        open(owner = 'modal'): string {
            nextId += 1
            const id = `${owner}-${nextId}`
            this.stack.push({ id, owner })
            return id
        },

        /** Removes dialog `id` wherever it is in the stack. Unknown ids are ignored. */
        close(id: string) {
            this.stack = this.stack.filter(entry => entry.id !== id)
        },
    },
})
