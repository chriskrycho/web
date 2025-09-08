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

---

A couple of other notes that have come up while implementing:

Working with `minijinja` currently requires everything to implement `Serialize` to be able to be called with `Template::render`. The biggest place this shows up as a constraint in the current design is my use of a `BTreeMap` for the underlying structure of an `Archive`:

```rust
pub struct Archive<'p>(BTreeMap<Year, MonthMap<'p>>);

#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Copy, Clone, Serialize)]
pub struct Year {
   raw: u32,
}

type MonthMap<'p> = BTreeMap<Month, DayMap<'p>>;

type DayMap<'p> = BTreeMap<Day, BTreeSet<&'p Post<'p>>>;
```

This is a fairly dumb port of the logic from v5 today, which looks like this:

```ts
type Archive = Year[];

export interface Year {
   name: string;
   months: Month[];
}

export interface Month {
   name: string;
   days: Day[];
}

export interface Day {
   name: string;
   items: Item[];
}

type YearMap = Map<number, [string, MonthMap]>;
type MonthMap = Map<number, [string, DayMap]>;
type DayMap = Map<number, Item[]>;
```

The key *design goal* is to be able to iterate over Y/M/D in the template easily, *without* needing to do a bunch of logic in templates themselves, because that is a terrible, *terrible* experience.

---

Does `ViewOf` make sense? I think it does *not*, in that it actually makes things *worse* at the render site than does individually implementing.

I guess the alternative here would be to implement the actual data structures passed into the rendering layer as *all* being `ViewOf`.

```rust
struct BookView<'a> {
   book: &'a ViewOf(Book),
   archive: &'a ViewOf(Archive<'a>),
}
```

I do *not* love that. It feels like it’d be better to `impl<'a> Object for BookView<'a> { … }` instead, even though it’s a bit of boilerplate.

---
