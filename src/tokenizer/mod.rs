use unic_segment::{WordBoundIndices, Words};

pub mod caps;

pub trait Tokenize {
    fn word_bound_indices(&self) -> WordBoundIndices;
    fn words(&self) -> Words;
    fn word_indices(&self) -> WordBoundIndices;
}

impl Tokenize for str {
    fn word_bound_indices(&self) -> WordBoundIndices {
        WordBoundIndices::new(self)
    }

    fn words(&self) -> Words {
        Words::new(self, |s| s.chars().any(|ch| ch.is_alphanumeric()))
    }

    fn word_indices(&self) -> WordBoundIndices {
        // TODO: this should use a new thing called WordIndices
        WordBoundIndices::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let msg = "this is an ordinary sentence! \"This was quoted,\", an emoji: (😄), and\t a tab was there and a new line.\n Some extreme unicode; bismala: (﷽), in long form: بِسْمِ اللهِ الرَّحْمٰنِ الرَّحِيْمِ.";
        msg.word_bound_indices().for_each(|t| println!("{:?}", t));
        println!("{}", &msg);
    }

    #[test]
    fn word_indices() {
        let text = "these are  4 words, the number   counts as\ta 'word' but punctuation doesn't.";

        let tokens = text.word_indices().collect::<Vec<(usize, &str)>>();

        assert_eq!(tokens, &[
            (0, "these"),
            (6, "are"),
            (11, "4"),
            (13, "words"),
            (20, "the"),
            (24, "number"),
            (33, "counts"),
            (40, "as"),
            (43, "a"),
            (46, "word"),
            (52, "but"),
            (56, "punctuation"),
            (68, "doesn\'t")
        ]);
        println!("{}", &text);
    }
}
