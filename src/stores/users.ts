import { defineStore } from 'pinia'
import { documentDir, join } from '@tauri-apps/api/path'
import * as api from '@/services/api'
import { playNewUser } from '@/utils/sounds'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { useOpdsStore } from '@/stores/opds'
import { reportError } from '@/services/librarium-api'
import { errorCode, errorMessage, isLibraryUnavailable } from '@/utils/errors'

const STORAGE_KEY = 'librarium:currentUser'

/**
 * Users and the library root. Each user's library is the folder
 * `<libraryRoot>/<user>`.
 *
 * This store owns the link between "who is the current user" and "which
 * library is open": every change of user or library root ends in
 * `applyCurrentUser()`, which opens that user's library in the library store
 * and points the OPDS server at it. Views don't load the library themselves,
 * so the Discover, Device and Settings views always act on the current user's
 * folder.
 *
 * A library root that doesn't exist is never created by listing it (an
 * unmounted NAS share must not become an empty local folder):
 * - the default root (`~/Documents/Librarium`) missing just means a first
 *   run: no users yet, and creating the first user creates the folder;
 * - any other failure to list the users — a root chosen in Settings that is
 *   missing (`LIBRARY_UNAVAILABLE`), one Librarium may not read
 *   (`PERMISSION_DENIED`: on macOS, the privacy settings for network
 *   volumes), an SMB session that expired, an I/O error — means the users
 *   are *unknown*, not that there are none: `unavailable` is set, the user
 *   picker shows why (`error`, `errorCode`) with Retry and no "Add user",
 *   and the current user is kept for when the folder can be read again
 *   (forgetting it would log the user out).
 */
export const useUsersStore = defineStore('users', {
    state: () => ({
        users: [] as string[],
        currentUser: localStorage.getItem(STORAGE_KEY),
        libraryRoot: null as string | null,
        loading: false,
        /** Why the last action failed (listing or creating users), for people. */
        error: null as string | null,
        /** The backend's code for the last listing failure (`LIBRARY_UNAVAILABLE`, `PERMISSION_DENIED`…). */
        errorCode: null as string | null,
        /**
         * The users couldn't be listed: the root can't be reached or read.
         * The users are unknown (not "none"); `error`/`errorCode` say why.
         */
        unavailable: false,
    }),

    getters: {
        /** Whether the root is the default folder, which Librarium may create (first run). */
        usesDefaultRoot: (): boolean => !useSettingsStore().libraryRootOverride,

        /** The current user's library folder, or null while no user is selected. */
        currentUserPath: (state): string | null => {
            if (!state.libraryRoot || !state.currentUser) { return null }
            // path.join is async, so this is built by hand; Rust accepts `/` on every OS.
            return `${state.libraryRoot}/${state.currentUser}`
        },

        hasUsers: (state): boolean => state.users.length > 0,
    },

    actions: {
        /**
         * Opens the current user's library and serves it over OPDS (or closes
         * both while no user is selected). Callers don't wait for it: a scan
         * of a large NAS library can take a while, and the library store
         * reports its own errors.
         */
        applyCurrentUser(): Promise<void> {
            const path = this.currentUserPath
            void useOpdsStore().setLibraryRoot(path)
            return useLibraryStore().openLibrary(path)
        },

        setCurrentUser(name: string | null) {
            this.currentUser = name
            if (name) {
                localStorage.setItem(STORAGE_KEY, name)
            } else {
                localStorage.removeItem(STORAGE_KEY)
            }
        },

        /**
         * Resolves the library root and the user list. Called at startup and when
         * the library folder is changed in Settings; either way the library is
         * (re)opened for whoever is current afterwards.
         */
        async init() {
            try {
                const settings = useSettingsStore()
                this.libraryRoot = settings.libraryRootOverride ?? await join(await documentDir(), 'Librarium')
                await this.loadUsers()
            } catch (e) {
                reportError('USERS_INIT_FAILED', 'users.init', e)
                throw e
            }

            // Always show the picker on startup when several users exist. While
            // the root is unreachable the users are unknown: keep the current one
            // (its library view shows "not reachable" with Retry).
            const knownUsers = !this.unavailable
            if (knownUsers && this.users.length > 1) {
                this.setCurrentUser(null)
            } else if (knownUsers && this.currentUser && !this.users.includes(this.currentUser)) {
                this.setCurrentUser(null)
            }
            void this.applyCurrentUser()
        },

        async loadUsers() {
            if (!this.libraryRoot) { return }
            this.loading = true
            this.error = null
            this.errorCode = null
            try {
                this.users = await api.listUsers(this.libraryRoot)
                this.unavailable = false
            } catch (e) {
                this.users = []
                if (isLibraryUnavailable(e) && this.usesDefaultRoot) {
                    // Default root missing: a first run, not an error.
                    this.unavailable = false
                } else {
                    // Anything else: the users are unknown. Never "no users",
                    // which would offer to create one in a folder that can't
                    // be read and forget the current user.
                    this.unavailable = true
                    this.error = errorMessage(e)
                    this.errorCode = errorCode(e)
                }
            } finally {
                this.loading = false
            }
        },

        /**
         * Retry after "library not reachable": re-reads the users, then reopens
         * the current user's library (if any).
         */
        async retry(): Promise<void> {
            await this.loadUsers()
            await this.applyCurrentUser()
        },

        /** Creates a user folder. Resolves with its name, or null (the reason is in `error`). */
        async createUser(name: string): Promise<string | null> {
            if (!this.libraryRoot) { return null }
            this.error = null
            try {
                // Only the default folder may be created (first run).
                const created = await api.createUser(this.libraryRoot, name, this.usesDefaultRoot)
                this.users.push(created)
                playNewUser()
                return created
            } catch (e) {
                this.error = errorMessage(e)
                return null
            }
        },

        selectUser(name: string): Promise<void> {
            this.setCurrentUser(name)
            return this.applyCurrentUser()
        },

        /** Back to the user picker; the previous user's library is closed. */
        clearUser(): Promise<void> {
            this.setCurrentUser(null)
            return this.applyCurrentUser()
        },
    },
})
