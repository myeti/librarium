import type { StoreBook, StoreProvider } from './types'
import { gutendexProvider } from './providers/gutendex'
import { openLibraryStoreProvider } from './providers/openLibrary'

export type { StoreBook, StoreProvider }

export const STORE_PROVIDERS: StoreProvider[] = [
    gutendexProvider,
    openLibraryStoreProvider,
]
