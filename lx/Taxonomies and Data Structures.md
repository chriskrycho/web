# Taxonomies and Data Structures

There are a couple different kinds of structure I want to support, and I need to handle them distinctly.

- **“Page”-like.** About, Contact, CV, etc. on v5; but also (maybe?) works pages on music.

- **“Post”-like.** Journal, Essays, Library posts, and Notes on v5. Also includes photo posts in v5. Distinguishing feature: They have dates!

    These are not all *the same*, but being “a thing with a date” is a commonality. That suggests something important, which I noticed long ago when thinking about this: these kinds of taxonomies may have *embedded* hierarchies but should not be hierarchical across each other. _I.e._, date-like should be one taxonomical structure, but there may be others as well: categorization/tagging, membership in some collection, series

- **Series.** All the writing about <cite>Fanfare for a New Era of American Spaceflight</cite> for example.

- **“Connected” to other things.** Basically, today, “library” posts. There should be a library *section*, with books (and other things? Or maybe not: maybe just books) as the “root”. Then posts can hang off of those. But also, per my note above, be surfaced as .

- **Links.** Link blogging is not my dominant mode, but it is *a* mode I work in sometimes.

- **“Works” of various sorts.** Photos, musical works, poetry. Maybe these do or maybe they *don’t* have anything in particular in common?

---

Data structures: I think what I basically want are *structured views* of the underlying data. The *core* data should simply be:

- all the pages that exist on the site
- all the distinct taxonomical “roots” that exist, I think?

Then the “views” provide whatever structure on top of them, possibly composing different taxonomies together. E.g., an `Archive` is a view that provides some ordered structure on top of the set of all items with dates, potentially filtered to some *other* taxonomy. Interesting: I may also want to have an archive *attached* to a non-archive view. For example, a book *page* may want to pull both the (non-`Archive`, because non-dated!) `Book` data *and* an `Archive` of all the posts attached to that `Book`.
