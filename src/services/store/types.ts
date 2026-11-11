/** The formats a Discover result can be downloaded in: the library's, which `download_and_import` accepts (the file is named `<…>.<format>`). */
export type StoreFormat = 'epub' | 'pdf' | 'mobi' | 'azw3' | 'cbz'

export interface StoreBook {
    id:         string      // unique per provider, e.g. "gutendex-84"
    source:     string      // provider id
    /** '' when the catalog doesn't say (the view shows a translated placeholder). */
    title:      string
    /** '' when unknown, like `title`. */
    author:     string
    language?:  string
    subjects?:  string[]
    coverUrl?:  string
    downloadUrl: string
    format:     StoreFormat
    sourceUrl?: string
}

export interface StoreProvider {
    readonly id: string
    readonly label: string
    /** Return false to skip this provider (e.g. missing API key). */
    isAvailable(): boolean
    search(query: string, signal?: AbortSignal): Promise<StoreBook[]>
}
