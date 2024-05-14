# Pluggable routing

A 'unique property' is a type that implements `FromRequest` and `Eq`.

A route may have any number of 'unique properties', and routes A and B
are considered colliding if they have colliding paths, and they do not
share a 'unique property' type with different values.

E.g.: `Format(MediaType)`.


