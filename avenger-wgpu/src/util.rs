#[macro_export]
macro_rules! par_izip {
    // Define the closure for flattening the tuple in a map call for parallel processing.
    ( @closure $p:pat => $tup:expr ) => {
        |$p| $tup
    };

    ( @closure $p:pat => ( $($tup:tt)* ) , $_iter:expr $( , $tail:expr )* ) => {
        par_izip!(@closure ($p, b) => ( $($tup)*, b ) $( , $tail )*)
    };

    // Unary case: Convert a single iterable into a parallel iterator.
    ($first:expr $(,)*) => {
        rayon::prelude::IntoParallelRefIterator::par_iter(&$first)
    };

    // Binary case: Zip two parallel iterators.
    ($first:expr, $second:expr $(,)*) => {
        par_izip!($first)
            .zip($second)
    };

    // N-ary case: Zip multiple parallel iterators and flatten the tuple.
    ( $first:expr $( , $rest:expr )* $(,)* ) => {
        par_izip!($first)
            $(
                .zip($rest)
            )*
            .map(
                par_izip!(@closure a => (a) $( , $rest )*)
            )
    };
}


#[cfg(test)]
mod tests {
    use rayon::prelude::*;

    #[test]
    fn try_it() {
        let v = par_izip!(vec![1, 2, 3], vec![1, 2, 3], vec![1, 2, 3])
            .map(|(a, b, c)| {
                println!("{a}, {b}, {c}");
                a + b + c
            })
            .collect::<Vec<_>>();
        println!("{v:?}");
    }
}
