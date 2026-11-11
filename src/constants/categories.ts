export const CATEGORIES = [
    'Adventure',
    'Art',
    'Biography & Memoir',
    'Business',
    "Children's",
    'Comics & Manga',
    'Cooking',
    'Fantasy',
    'Fiction',
    'Health',
    'Historical Fiction',
    'History',
    'Horror',
    'Mystery & Thriller',
    'Nature',
    'Philosophy',
    'Poetry',
    'Politics',
    'Psychology',
    'Religion',
    'Romance',
    'Science',
    'Science Fiction',
    'Self-Help',
    'Technology',
    'Travel',
    'Young Adult',
    'Other',
] as const

export type Category = (typeof CATEGORIES)[number]

// More specific patterns first, fiction subgenres before generic "fiction"
const KEYWORD_MAP: [string[], string][] = [
    [['science fiction', 'sci-fi', 'space opera', 'cyberpunk', 'dystopian', 'dystopia', 'speculative fiction'], 'Science Fiction'],
    [['fantasy', 'magic', 'sword and sorcery', 'high fantasy', 'epic fantasy'], 'Fantasy'],
    [['mystery', 'detective', 'crime fiction', 'thriller', 'suspense', 'noir', 'whodunit'], 'Mystery & Thriller'],
    [['horror', 'ghost stories', 'supernatural fiction', 'gothic fiction'], 'Horror'],
    [['romance', 'love stories', 'romantic fiction'], 'Romance'],
    [['historical fiction', 'historical novel', 'historical romance'], 'Historical Fiction'],
    [['young adult', 'teen fiction', 'ya fiction'], 'Young Adult'],
    [['juvenile', "children's", 'picture book', 'fairy tales', "children's stories"], "Children's"],
    [['adventure', 'action fiction'], 'Adventure'],
    [['comic', 'manga', 'graphic novel', 'bande dessinée'], 'Comics & Manga'],
    [['biography', 'autobiography', 'memoir', 'life stories'], 'Biography & Memoir'],
    [['technology', 'computer science', 'programming', 'engineering', 'software', 'artificial intelligence'], 'Technology'],
    [['science', 'physics', 'chemistry', 'biology', 'astronomy', 'mathematics', 'cosmology', 'evolution'], 'Science'],
    [['history', 'world war', 'ancient history', 'civilization', 'archaeology'], 'History'],
    [['philosophy', 'ethics', 'epistemology', 'metaphysics', 'logic'], 'Philosophy'],
    [['psychology', 'psychiatry', 'cognitive science', 'behavioral'], 'Psychology'],
    [['business', 'economics', 'finance', 'management', 'entrepreneurship', 'investing', 'marketing'], 'Business'],
    [['self-help', 'self help', 'personal development', 'motivation', 'productivity', 'leadership'], 'Self-Help'],
    [['health', 'medicine', 'nutrition', 'fitness', 'wellness', 'anatomy'], 'Health'],
    [['politics', 'political science', 'government', 'democracy', 'geopolitics'], 'Politics'],
    [['travel', 'geography', 'exploration', 'adventure travel'], 'Travel'],
    [['art', 'design', 'architecture', 'painting', 'drawing', 'illustration', 'photography'], 'Art'],
    [['religion', 'spirituality', 'theology', 'christianity', 'islam', 'buddhism', 'hinduism', 'bible'], 'Religion'],
    [['cooking', 'cookery', 'recipes', 'cuisine', 'baking', 'gastronomy'], 'Cooking'],
    [['nature', 'environment', 'ecology', 'wildlife', 'animals', 'botany', 'conservation'], 'Nature'],
    [['poetry', 'poems', 'verse', 'poetics'], 'Poetry'],
    [['fiction', 'novel', 'short stories', 'literature'], 'Fiction'],
]

export function detectCategory(subjects: string[]): string | undefined {
    const normalized = subjects.map(s => s.toLowerCase())
    for (const [keywords, category] of KEYWORD_MAP) {
        for (const subject of normalized) {
            for (const keyword of keywords) {
                if (subject.includes(keyword)) {return category}
            }
        }
    }
    return undefined
}
