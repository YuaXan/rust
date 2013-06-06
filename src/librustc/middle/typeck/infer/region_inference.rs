// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/*!

Region inference module.

# Introduction

Region inference uses a somewhat more involved algorithm than type
inference.  It is not the most efficient thing ever written though it
seems to work well enough in practice (famous last words).  The reason
that we use a different algorithm is because, unlike with types, it is
impractical to hand-annotate with regions (in some cases, there aren't
even the requisite syntactic forms).  So we have to get it right, and
it's worth spending more time on a more involved analysis.  Moreover,
regions are a simpler case than types: they don't have aggregate
structure, for example.

Unlike normal type inference, which is similar in spirit to H-M and thus
works progressively, the region type inference works by accumulating
constraints over the course of a function.  Finally, at the end of
processing a function, we process and solve the constraints all at
once.

The constraints are always of one of three possible forms:

- ConstrainVarSubVar(R_i, R_j) states that region variable R_i
  must be a subregion of R_j
- ConstrainRegSubVar(R, R_i) states that the concrete region R
  (which must not be a variable) must be a subregion of the varibale R_i
- ConstrainVarSubReg(R_i, R) is the inverse

# Building up the constraints

Variables and constraints are created using the following methods:

- `new_region_var()` creates a new, unconstrained region variable;
- `make_subregion(R_i, R_j)` states that R_i is a subregion of R_j
- `lub_regions(R_i, R_j) -> R_k` returns a region R_k which is
  the smallest region that is greater than both R_i and R_j
- `glb_regions(R_i, R_j) -> R_k` returns a region R_k which is
  the greatest region that is smaller than both R_i and R_j

The actual region resolution algorithm is not entirely
obvious, though it is also not overly complex.  I'll explain
the algorithm as it currently works, then explain a somewhat
more complex variant that would probably scale better for
large graphs (and possibly all graphs).

## Snapshotting

It is also permitted to try (and rollback) changes to the graph.  This
is done by invoking `start_snapshot()`, which returns a value.  Then
later you can call `rollback_to()` which undoes the work.
Alternatively, you can call `commit()` which ends all snapshots.
Snapshots can be recursive---so you can start a snapshot when another
is in progress, but only the root snapshot can "commit".

# Resolving constraints

The constraint resolution algorithm is not super complex but also not
entirely obvious.  Here I describe the problem somewhat abstractly,
then describe how the current code works, and finally describe a
better solution that is as of yet unimplemented.  There may be other,
smarter ways of doing this with which I am unfamiliar and can't be
bothered to research at the moment. - NDM

## The problem

Basically our input is a directed graph where nodes can be divided
into two categories: region variables and concrete regions.  Each edge
`R -> S` in the graph represents a constraint that the region `R` is a
subregion of the region `S`.

Region variable nodes can have arbitrary degree.  There is one region
variable node per region variable.

Each concrete region node is associated with some, well, concrete
region: e.g., a free lifetime, or the region for a particular scope.
Note that there may be more than one concrete region node for a
particular region value.  Moreover, because of how the graph is built,
we know that all concrete region nodes have either in-degree 1 or
out-degree 1.

Before resolution begins, we build up the constraints in a hashmap
that maps `Constraint` keys to spans.  During resolution, we construct
the actual `Graph` structure that we describe here.

## Our current algorithm

We divide region variables into two groups: Expanding and Contracting.
Expanding region variables are those that have a concrete region
predecessor (direct or indirect).  Contracting region variables are
all others.

We first resolve the values of Expanding region variables and then
process Contracting ones.  We currently use an iterative, fixed-point
procedure (but read on, I believe this could be replaced with a linear
walk).  Basically we iterate over the edges in the graph, ensuring
that, if the source of the edge has a value, then this value is a
subregion of the target value.  If the target does not yet have a
value, it takes the value from the source.  If the target already had
a value, then the resulting value is Least Upper Bound of the old and
new values. When we are done, each Expanding node will have the
smallest region that it could possibly have and still satisfy the
constraints.

We next process the Contracting nodes.  Here we again iterate over the
edges, only this time we move values from target to source (if the
source is a Contracting node).  For each contracting node, we compute
its value as the GLB of all its successors.  Basically contracting
nodes ensure that there is overlap between their successors; we will
ultimately infer the largest overlap possible.

### A better algorithm

Fixed-point iteration is not necessary.  What we ought to do is first
identify and remove strongly connected components (SCC) in the graph.
Note that such components must consist solely of region variables; all
of these variables can effectively be unified into a single variable.

Once SCCs are removed, we are left with a DAG.  At this point, we can
walk the DAG in toplogical order once to compute the expanding nodes,
and again in reverse topological order to compute the contracting
nodes. The main reason I did not write it this way is that I did not
feel like implementing the SCC and toplogical sort algorithms at the
moment.

# Skolemization and functions

One of the trickiest and most subtle aspects of regions is dealing
with the fact that region variables are bound in function types.  I
strongly suggest that if you want to understand the situation, you
read this paper (which is, admittedly, very long, but you don't have
to read the whole thing):

http://research.microsoft.com/en-us/um/people/simonpj/papers/higher-rank/

Although my explanation will never compete with SPJ's (for one thing,
his is approximately 100 pages), I will attempt to explain the basic
problem and also how we solve it.  Note that the paper only discusses
subtyping, not the computation of LUB/GLB.

The problem we are addressing is that there is a kind of subtyping
between functions with bound region parameters.  Consider, for
example, whether the following relation holds:

    fn(&'a int) <: &fn(&'b int)? (Yes, a => b)

The answer is that of course it does.  These two types are basically
the same, except that in one we used the name `a` and one we used
the name `b`.

In the examples that follow, it becomes very important to know whether
a lifetime is bound in a function type (that is, is a lifetime
parameter) or appears free (is defined in some outer scope).
Therefore, from now on I will write the bindings explicitly, using a
notation like `fn<a>(&'a int)` to indicate that `a` is a lifetime
parameter.

Now let's consider two more function types.  Here, we assume that the
`self` lifetime is defined somewhere outside and hence is not a
lifetime parameter bound by the function type (it "appears free"):

    fn<a>(&'a int) <: &fn(&'self int)? (Yes, a => self)

This subtyping relation does in fact hold.  To see why, you have to
consider what subtyping means.  One way to look at `T1 <: T2` is to
say that it means that it is always ok to treat an instance of `T1` as
if it had the type `T2`.  So, with our functions, it is always ok to
treat a function that can take pointers with any lifetime as if it
were a function that can only take a pointer with the specific
lifetime `&self`.  After all, `&self` is a lifetime, after all, and
the function can take values of any lifetime.

You can also look at subtyping as the *is a* relationship.  This amounts
to the same thing: a function that accepts pointers with any lifetime
*is a* function that accepts pointers with some specific lifetime.

So, what if we reverse the order of the two function types, like this:

    fn(&'self int) <: &fn<a>(&'a int)? (No)

Does the subtyping relationship still hold?  The answer of course is
no.  In this case, the function accepts *only the lifetime `&self`*,
so it is not reasonable to treat it as if it were a function that
accepted any lifetime.

What about these two examples:

    fn<a,b>(&'a int, &'b int) <: &fn<a>(&'a int, &'a int)? (Yes)
    fn<a>(&'a int, &'a int) <: &fn<a,b>(&'a int, &'b int)? (No)

Here, it is true that functions which take two pointers with any two
lifetimes can be treated as if they only accepted two pointers with
the same lifetime, but not the reverse.

## The algorithm

Here is the algorithm we use to perform the subtyping check:

1. Replace all bound regions in the subtype with new variables
2. Replace all bound regions in the supertype with skolemized
   equivalents.  A "skolemized" region is just a new fresh region
   name.
3. Check that the parameter and return types match as normal
4. Ensure that no skolemized regions 'leak' into region variables
   visible from "the outside"

Let's walk through some examples and see how this algorithm plays out.

#### First example

We'll start with the first example, which was:

    1. fn<a>(&'a T) <: &fn<b>(&'b T)?        Yes: a -> b

After steps 1 and 2 of the algorithm we will have replaced the types
like so:

    1. fn(&'A T) <: &fn(&'x T)?

Here the upper case `&A` indicates a *region variable*, that is, a
region whose value is being inferred by the system.  I also replaced
`&b` with `&x`---I'll use letters late in the alphabet (`x`, `y`, `z`)
to indicate skolemized region names.  We can assume they don't appear
elsewhere.  Note that neither the sub- nor the supertype bind any
region names anymore (as indicated by the absence of `<` and `>`).

The next step is to check that the parameter types match.  Because
parameters are contravariant, this means that we check whether:

    &'x T <: &'A T

Region pointers are contravariant so this implies that

    &A <= &x

must hold, where `<=` is the subregion relationship.  Processing
*this* constrain simply adds a constraint into our graph that `&A <=
&x` and is considered successful (it can, for example, be satisfied by
choosing the value `&x` for `&A`).

So far we have encountered no error, so the subtype check succeeds.

#### The third example

Now let's look first at the third example, which was:

    3. fn(&'self T)    <: &fn<b>(&'b T)?        No!

After steps 1 and 2 of the algorithm we will have replaced the types
like so:

    3. fn(&'self T) <: &fn(&'x T)?

This looks pretty much the same as before, except that on the LHS
`&self` was not bound, and hence was left as-is and not replaced with
a variable.  The next step is again to check that the parameter types
match.  This will ultimately require (as before) that `&self` <= `&x`
must hold: but this does not hold.  `self` and `x` are both distinct
free regions.  So the subtype check fails.

#### Checking for skolemization leaks

You may be wondering about that mysterious last step in the algorithm.
So far it has not been relevant.  The purpose of that last step is to
catch something like *this*:

    fn<a>() -> fn(&'a T) <: &fn() -> fn<b>(&'b T)?   No.

Here the function types are the same but for where the binding occurs.
The subtype returns a function that expects a value in precisely one
region.  The supertype returns a function that expects a value in any
region.  If we allow an instance of the subtype to be used where the
supertype is expected, then, someone could call the fn and think that
the return value has type `fn<b>(&'b T)` when it really has type
`fn(&'a T)` (this is case #3, above).  Bad.

So let's step through what happens when we perform this subtype check.
We first replace the bound regions in the subtype (the supertype has
no bound regions).  This gives us:

    fn() -> fn(&'A T) <: &fn() -> fn<b>(&'b T)?

Now we compare the return types, which are covariant, and hence we have:

    fn(&'A T) <: &fn<b>(&'b T)?

Here we skolemize the bound region in the supertype to yield:

    fn(&'A T) <: &fn(&'x T)?

And then proceed to compare the argument types:

    &'x T <: &'A T
    &A <= &x

Finally, this is where it gets interesting!  This is where an error
*should* be reported.  But in fact this will not happen.  The reason why
is that `A` is a variable: we will infer that its value is the fresh
region `x` and think that everything is happy.  In fact, this behavior
is *necessary*, it was key to the first example we walked through.

The difference between this example and the first one is that the variable
`A` already existed at the point where the skolemization occurred.  In
the first example, you had two functions:

    fn<a>(&'a T) <: &fn<b>(&'b T)

and hence `&A` and `&x` were created "together".  In general, the
intention of the skolemized names is that they are supposed to be
fresh names that could never be equal to anything from the outside.
But when inference comes into play, we might not be respecting this
rule.

So the way we solve this is to add a fourth step that examines the
constraints that refer to skolemized names.  Basically, consider a
non-directed verison of the constraint graph.  Let `Tainted(x)` be the
set of all things reachable from a skolemized variable `x`.
`Tainted(x)` should not contain any regions that existed before the
step at which the skolemization was performed.  So this case here
would fail because `&x` was created alone, but is relatable to `&A`.

## Computing the LUB and GLB

The paper I pointed you at is written for Haskell.  It does not
therefore considering subtyping and in particular does not consider
LUB or GLB computation.  We have to consider this.  Here is the
algorithm I implemented.

First though, let's discuss what we are trying to compute in more
detail.  The LUB is basically the "common supertype" and the GLB is
"common subtype"; one catch is that the LUB should be the
*most-specific* common supertype and the GLB should be *most general*
common subtype (as opposed to any common supertype or any common
subtype).

Anyway, to help clarify, here is a table containing some
function pairs and their LUB/GLB:

```
Type 1              Type 2              LUB               GLB
fn<a>(&a)           fn(&X)              fn(&X)            fn<a>(&a)
fn(&A)              fn(&X)              --                fn<a>(&a)
fn<a,b>(&a, &b)     fn<x>(&x, &x)       fn<a>(&a, &a)     fn<a,b>(&a, &b)
fn<a,b>(&a, &b, &a) fn<x,y>(&x, &y, &y) fn<a>(&a, &a, &a) fn<a,b,c>(&a,&b,&c)
```

### Conventions

I use lower-case letters (e.g., `&a`) for bound regions and upper-case
letters for free regions (`&A`).  Region variables written with a
dollar-sign (e.g., `$a`).  I will try to remember to enumerate the
bound-regions on the fn type as well (e.g., `fn<a>(&a)`).

### High-level summary

Both the LUB and the GLB algorithms work in a similar fashion.  They
begin by replacing all bound regions (on both sides) with fresh region
inference variables.  Therefore, both functions are converted to types
that contain only free regions.  We can then compute the LUB/GLB in a
straightforward way, as described in `combine.rs`.  This results in an
interim type T.  The algorithms then examine the regions that appear
in T and try to, in some cases, replace them with bound regions to
yield the final result.

To decide whether to replace a region `R` that appears in `T` with a
bound region, the algorithms make use of two bits of information.
First is a set `V` that contains all region variables created as part
of the LUB/GLB computation. `V` will contain the region variables
created to replace the bound regions in the input types, but it also
contains 'intermediate' variables created to represent the LUB/GLB of
individual regions.  Basically, when asked to compute the LUB/GLB of a
region variable with another region, the inferencer cannot oblige
immediately since the valuese of that variables are not known.
Therefore, it creates a new variable that is related to the two
regions.  For example, the LUB of two variables `$x` and `$y` is a
fresh variable `$z` that is constrained such that `$x <= $z` and `$y
<= $z`.  So `V` will contain these intermediate variables as well.

The other important factor in deciding how to replace a region in T is
the function `Tainted($r)` which, for a region variable, identifies
all regions that the region variable is related to in some way
(`Tainted()` made an appearance in the subtype computation as well).

### LUB

The LUB algorithm proceeds in three steps:

1. Replace all bound regions (on both sides) with fresh region
   inference variables.
2. Compute the LUB "as normal", meaning compute the GLB of each
   pair of argument types and the LUB of the return types and
   so forth.  Combine those to a new function type `F`.
3. Replace each region `R` that appears in `F` as follows:
   - Let `V` be the set of variables created during the LUB
     computational steps 1 and 2, as described in the previous section.
   - If `R` is not in `V`, replace `R` with itself.
   - If `Tainted(R)` contains a region that is not in `V`,
     replace `R` with itself.
   - Otherwise, select the earliest variable in `Tainted(R)` that originates
     from the left-hand side and replace `R` with the bound region that
     this variable was a replacement for.

So, let's work through the simplest example: `fn(&A)` and `fn<a>(&a)`.
In this case, `&a` will be replaced with `$a` and the interim LUB type
`fn($b)` will be computed, where `$b=GLB(&A,$a)`.  Therefore, `V =
{$a, $b}` and `Tainted($b) = { $b, $a, &A }`.  When we go to replace
`$b`, we find that since `&A \in Tainted($b)` is not a member of `V`,
we leave `$b` as is.  When region inference happens, `$b` will be
resolved to `&A`, as we wanted.

Let's look at a more complex one: `fn(&a, &b)` and `fn(&x, &x)`.  In
this case, we'll end up with a (pre-replacement) LUB type of `fn(&g,
&h)` and a graph that looks like:

```
     $a        $b     *--$x
       \        \    /  /
        \        $h-*  /
         $g-----------*
```

Here `$g` and `$h` are fresh variables that are created to represent
the LUB/GLB of things requiring inference.  This means that `V` and
`Tainted` will look like:

```
V = {$a, $b, $g, $h, $x}
Tainted($g) = Tainted($h) = { $a, $b, $h, $g, $x }
```

Therefore we replace both `$g` and `$h` with `$a`, and end up
with the type `fn(&a, &a)`.

### GLB

The procedure for computing the GLB is similar.  The difference lies
in computing the replacements for the various variables. For each
region `R` that appears in the type `F`, we again compute `Tainted(R)`
and examine the results:

1. If `R` is not in `V`, it is not replaced.
2. Else, if `Tainted(R)` contains only variables in `V`, and it
   contains exactly one variable from the LHS and one variable from
   the RHS, then `R` can be mapped to the bound version of the
   variable from the LHS.
3. Else, if `Tainted(R)` contains no variable from the LHS and no
   variable from the RHS, then `R` can be mapped to itself.
4. Else, `R` is mapped to a fresh bound variable.

These rules are pretty complex.  Let's look at some examples to see
how they play out.

Out first example was `fn(&a)` and `fn(&X)`.  In this case, `&a` will
be replaced with `$a` and we will ultimately compute a
(pre-replacement) GLB type of `fn($g)` where `$g=LUB($a,&X)`.
Therefore, `V={$a,$g}` and `Tainted($g)={$g,$a,&X}.  To find the
replacement for `$g` we consult the rules above:
- Rule (1) does not apply because `$g \in V`
- Rule (2) does not apply because `&X \in Tainted($g)`
- Rule (3) does not apply because `$a \in Tainted($g)`
- Hence, by rule (4), we replace `$g` with a fresh bound variable `&z`.
So our final result is `fn(&z)`, which is correct.

The next example is `fn(&A)` and `fn(&Z)`. In this case, we will again
have a (pre-replacement) GLB of `fn(&g)`, where `$g = LUB(&A,&Z)`.
Therefore, `V={$g}` and `Tainted($g) = {$g, &A, &Z}`.  In this case,
by rule (3), `$g` is mapped to itself, and hence the result is
`fn($g)`.  This result is correct (in this case, at least), but it is
indicative of a case that *can* lead us into concluding that there is
no GLB when in fact a GLB does exist.  See the section "Questionable
Results" below for more details.

The next example is `fn(&a, &b)` and `fn(&c, &c)`. In this case, as
before, we'll end up with `F=fn($g, $h)` where `Tainted($g) =
Tainted($h) = {$g, $h, $a, $b, $c}`.  Only rule (4) applies and hence
we'll select fresh bound variables `y` and `z` and wind up with
`fn(&y, &z)`.

For the last example, let's consider what may seem trivial, but is
not: `fn(&a, &a)` and `fn(&b, &b)`.  In this case, we'll get `F=fn($g,
$h)` where `Tainted($g) = {$g, $a, $x}` and `Tainted($h) = {$h, $a,
$x}`.  Both of these sets contain exactly one bound variable from each
side, so we'll map them both to `&a`, resulting in `fn(&a, &a)`, which
is the desired result.

### Shortcomings and correctness

You may be wondering whether this algorithm is correct.  The answer is
"sort of".  There are definitely cases where they fail to compute a
result even though a correct result exists.  I believe, though, that
if they succeed, then the result is valid, and I will attempt to
convince you.  The basic argument is that the "pre-replacement" step
computes a set of constraints.  The replacements, then, attempt to
satisfy those constraints, using bound identifiers where needed.

For now I will briefly go over the cases for LUB/GLB and identify
their intent:

- LUB:
  - The region variables that are substituted in place of bound regions
    are intended to collect constraints on those bound regions.
  - If Tainted(R) contains only values in V, then this region is unconstrained
    and can therefore be generalized, otherwise it cannot.
- GLB:
  - The region variables that are substituted in place of bound regions
    are intended to collect constraints on those bound regions.
  - If Tainted(R) contains exactly one variable from each side, and
    only variables in V, that indicates that those two bound regions
    must be equated.
  - Otherwise, if Tainted(R) references any variables from left or right
    side, then it is trying to combine a bound region with a free one or
    multiple bound regions, so we need to select fresh bound regions.

Sorry this is more of a shorthand to myself.  I will try to write up something
more convincing in the future.

#### Where are the algorithms wrong?

- The pre-replacement computation can fail even though using a
  bound-region would have succeeded.
- We will compute GLB(fn(fn($a)), fn(fn($b))) as fn($c) where $c is the
  GLB of $a and $b.  But if inference finds that $a and $b must be mapped
  to regions without a GLB, then this is effectively a failure to compute
  the GLB.  However, the result `fn<$c>(fn($c))` is a valid GLB.

*/


use middle::ty;
use middle::ty::{FreeRegion, Region, RegionVid};
use middle::ty::{re_empty, re_static, re_infer, re_free, re_bound};
use middle::ty::{re_scope, ReVar, ReSkolemized, br_fresh};
use middle::typeck::infer::cres;
use middle::typeck::infer::{RegionVariableOrigin, SubregionOrigin};
use middle::typeck::infer;
use util::common::indenter;
use util::ppaux::{note_and_explain_region, Repr, UserString};

use std::cell::Cell;
use std::hashmap::{HashMap, HashSet};
use std::uint;
use std::vec;
use syntax::codemap::span;
use syntax::ast;
use syntax::opt_vec;
use syntax::opt_vec::OptVec;

#[deriving(Eq,IterBytes)]
enum Constraint {
    ConstrainVarSubVar(RegionVid, RegionVid),
    ConstrainRegSubVar(Region, RegionVid),
    ConstrainVarSubReg(RegionVid, Region)
}

#[deriving(Eq, IterBytes)]
struct TwoRegions {
    a: Region,
    b: Region,
}

enum UndoLogEntry {
    Snapshot,
    AddVar(RegionVid),
    AddConstraint(Constraint),
    AddCombination(CombineMapType, TwoRegions)
}

enum CombineMapType {
    Lub, Glb
}

pub enum RegionResolutionError {
    /// `ConcreteFailure(o, a, b)`:
    ///
    /// `o` requires that `a <= b`, but this does not hold
    ConcreteFailure(SubregionOrigin, Region, Region),

    /// `SubSupConflict(v, sub_origin, sub_r, sup_origin, sup_r)`:
    ///
    /// Could not infer a value for `v` because `sub_r <= v` (due to
    /// `sub_origin`) but `v <= sup_r` (due to `sup_origin`) and
    /// `sub_r <= sup_r` does not hold.
    SubSupConflict(RegionVariableOrigin,
                   SubregionOrigin, Region,
                   SubregionOrigin, Region),

    /// `SupSupConflict(v, origin1, r1, origin2, r2)`:
    ///
    /// Could not infer a value for `v` because `v <= r1` (due to
    /// `origin1`) and `v <= r2` (due to `origin2`) and
    /// `r1` and `r2` have no intersection.
    SupSupConflict(RegionVariableOrigin,
                   SubregionOrigin, Region,
                   SubregionOrigin, Region),
}

type CombineMap = HashMap<TwoRegions, RegionVid>;

pub struct RegionVarBindings {
    tcx: ty::ctxt,
    var_origins: ~[RegionVariableOrigin],
    constraints: HashMap<Constraint, SubregionOrigin>,
    lubs: CombineMap,
    glbs: CombineMap,
    skolemization_count: uint,
    bound_count: uint,

    // The undo log records actions that might later be undone.
    //
    // Note: when the undo_log is empty, we are not actively
    // snapshotting.  When the `start_snapshot()` method is called, we
    // push a Snapshot entry onto the list to indicate that we are now
    // actively snapshotting.  The reason for this is that otherwise
    // we end up adding entries for things like the lower bound on
    // a variable and so forth, which can never be rolled back.
    undo_log: ~[UndoLogEntry],

    // This contains the results of inference.  It begins as an empty
    // cell and only acquires a value after inference is complete.
    // We use a cell vs a mutable option to circumvent borrowck errors.
    values: Cell<~[GraphNodeValue]>,
}

pub fn RegionVarBindings(tcx: ty::ctxt) -> RegionVarBindings {
    RegionVarBindings {
        tcx: tcx,
        var_origins: ~[],
        values: Cell::new_empty(),
        constraints: HashMap::new(),
        lubs: HashMap::new(),
        glbs: HashMap::new(),
        skolemization_count: 0,
        bound_count: 0,
        undo_log: ~[]
    }
}

impl RegionVarBindings {
    pub fn in_snapshot(&self) -> bool {
        self.undo_log.len() > 0
    }

    pub fn start_snapshot(&mut self) -> uint {
        debug!("RegionVarBindings: snapshot()=%u", self.undo_log.len());
        if self.in_snapshot() {
            self.undo_log.len()
        } else {
            self.undo_log.push(Snapshot);
            0
        }
    }

    pub fn commit(&mut self) {
        debug!("RegionVarBindings: commit()");
        while self.undo_log.len() > 0 {
            self.undo_log.pop();
        }
    }

    pub fn rollback_to(&mut self, snapshot: uint) {
        debug!("RegionVarBindings: rollback_to(%u)", snapshot);
        while self.undo_log.len() > snapshot {
            let undo_item = self.undo_log.pop();
            debug!("undo_item=%?", undo_item);
            match undo_item {
              Snapshot => {}
              AddVar(vid) => {
                assert_eq!(self.var_origins.len(), vid.to_uint() + 1);
                self.var_origins.pop();
              }
              AddConstraint(ref constraint) => {
                self.constraints.remove(constraint);
              }
              AddCombination(Glb, ref regions) => {
                self.glbs.remove(regions);
              }
              AddCombination(Lub, ref regions) => {
                self.lubs.remove(regions);
              }
            }
        }
    }

    pub fn num_vars(&mut self) -> uint {
        self.var_origins.len()
    }

    pub fn new_region_var(&mut self, origin: RegionVariableOrigin) -> RegionVid {
        let id = self.num_vars();
        self.var_origins.push(origin);
        let vid = RegionVid { id: id };
        if self.in_snapshot() {
            self.undo_log.push(AddVar(vid));
        }
        debug!("created new region variable %? with origin %?",
               vid, origin.repr(self.tcx));
        return vid;
    }

    pub fn new_skolemized(&mut self, br: ty::bound_region) -> Region {
        let sc = self.skolemization_count;
        self.skolemization_count += 1;
        re_infer(ReSkolemized(sc, br))
    }

    pub fn new_bound(&mut self) -> Region {
        // Creates a fresh bound variable for use in GLB computations.
        // See discussion of GLB computation in the large comment at
        // the top of this file for more details.
        //
        // This computation is mildly wrong in the face of rollover.
        // It's conceivable, if unlikely, that one might wind up with
        // accidental capture for nested functions in that case, if
        // the outer function had bound regions created a very long
        // time before and the inner function somehow wound up rolling
        // over such that supposedly fresh identifiers were in fact
        // shadowed.  We should convert our bound_region
        // representation to use deBruijn indices or something like
        // that to eliminate that possibility.

        let sc = self.bound_count;
        self.bound_count += 1;
        re_bound(br_fresh(sc))
    }

    pub fn add_constraint(&mut self,
                          constraint: Constraint,
                          origin: SubregionOrigin) {
        // cannot add constraints once regions are resolved
        assert!(self.values.is_empty());

        debug!("RegionVarBindings: add_constraint(%?)", constraint);

        if self.constraints.insert(constraint, origin) {
            if self.in_snapshot() {
                self.undo_log.push(AddConstraint(constraint));
            }
        }
    }

    pub fn make_subregion(&mut self,
                          origin: SubregionOrigin,
                          sub: Region,
                          sup: Region) {
        // cannot add constraints once regions are resolved
        assert!(self.values.is_empty());

        debug!("RegionVarBindings: make_subregion(%?, %?)", sub, sup);
        match (sub, sup) {
          (re_infer(ReVar(sub_id)), re_infer(ReVar(sup_id))) => {
            self.add_constraint(ConstrainVarSubVar(sub_id, sup_id), origin);
          }
          (r, re_infer(ReVar(sup_id))) => {
            self.add_constraint(ConstrainRegSubVar(r, sup_id), origin);
          }
          (re_infer(ReVar(sub_id)), r) => {
            self.add_constraint(ConstrainVarSubReg(sub_id, r), origin);
          }
          (re_bound(br), _) => {
            self.tcx.sess.span_bug(
                origin.span(),
                fmt!("Cannot relate bound region as subregion: %?", br));
          }
          (_, re_bound(br)) => {
            self.tcx.sess.span_bug(
                origin.span(),
                fmt!("Cannot relate bound region as superregion: %?", br));
          }
          _ => {
            self.add_constraint(ConstrainRegSubReg(sub, sup), origin);
          }
        }
    }

    pub fn lub_regions(&mut self,
                       origin: SubregionOrigin,
                       a: Region,
                       b: Region)
                       -> Region {
        // cannot add constraints once regions are resolved
        assert!(self.values.is_empty());

        debug!("RegionVarBindings: lub_regions(%?, %?)", a, b);
        match (a, b) {
            (re_static, _) | (_, re_static) => {
                re_static // nothing lives longer than static
            }

            _ => {
                self.combine_vars(
                    Lub, a, b, origin,
                    |this, old_r, new_r|
                    this.make_subregion(origin, old_r, new_r))
            }
        }
    }

    pub fn glb_regions(&mut self,
                       origin: SubregionOrigin,
                       a: Region,
                       b: Region)
                       -> Region {
        // cannot add constraints once regions are resolved
        assert!(self.values.is_empty());

        debug!("RegionVarBindings: glb_regions(%?, %?)", a, b);
        match (a, b) {
            (re_static, r) | (r, re_static) => {
                // static lives longer than everything else
                r
            }

            _ => {
                self.combine_vars(
                    Glb, a, b, origin,
                    |this, old_r, new_r|
                    this.make_subregion(origin, new_r, old_r))
            }
        }
    }

    pub fn resolve_var(&mut self, rid: RegionVid) -> ty::Region {
        if self.values.is_empty() {
            self.tcx.sess.span_bug(
                self.var_origins[rid.to_uint()].span(),
                fmt!("Attempt to resolve region variable before values have \
                      been computed!"));
        }

        let v = self.values.with_ref(|values| values[rid.to_uint()]);
        debug!("RegionVarBindings: resolve_var(%?=%u)=%?",
               rid, rid.to_uint(), v);
        match v {
            Value(r) => r,

            NoValue => {
                // No constraints, return ty::re_empty
                re_empty
            }

            ErrorValue => {
                // An error that has previously been reported.
                re_static
            }
        }
    }

    fn combine_map<'a>(&'a mut self,
                       t: CombineMapType)
                       -> &'a mut CombineMap
    {
        match t {
            Glb => &mut self.glbs,
            Lub => &mut self.lubs,
        }
    }

    pub fn combine_vars(&mut self,
                        t: CombineMapType,
                        a: Region,
                        b: Region,
                        origin: SubregionOrigin,
                        relate: &fn(this: &mut RegionVarBindings,
                                    old_r: Region,
                                    new_r: Region))
                        -> Region {
        let vars = TwoRegions { a: a, b: b };
        match self.combine_map(t).find(&vars) {
            Some(&c) => {
                return re_infer(ReVar(c));
            }
            None => {}
        }
        let c = self.new_region_var(infer::MiscVariable(origin.span()));
        self.combine_map(t).insert(vars, c);
        if self.in_snapshot() {
            self.undo_log.push(AddCombination(t, vars));
        }
        relate(self, a, re_infer(ReVar(c)));
        relate(self, b, re_infer(ReVar(c)));
        debug!("combine_vars() c=%?", c);
        re_infer(ReVar(c))
    }

    pub fn vars_created_since_snapshot(&mut self, snapshot: uint)
                                       -> ~[RegionVid] {
        do vec::build |push| {
            for uint::range(snapshot, self.undo_log.len()) |i| {
                match self.undo_log[i] {
                    AddVar(vid) => push(vid),
                    _ => ()
                }
            }
        }
    }

    pub fn tainted(&mut self, snapshot: uint, r0: Region) -> ~[Region] {
        /*!
         *
         * Computes all regions that have been related to `r0` in any
         * way since the snapshot `snapshot` was taken---`r0` itself
         * will be the first entry. This is used when checking whether
         * skolemized regions are being improperly related to other
         * regions.
         */

        debug!("tainted(snapshot=%u, r0=%?)", snapshot, r0);
        let _indenter = indenter();

        let undo_len = self.undo_log.len();

        // `result_set` acts as a worklist: we explore all outgoing
        // edges and add any new regions we find to result_set.  This
        // is not a terribly efficient implementation.
        let mut result_set = ~[r0];
        let mut result_index = 0;
        while result_index < result_set.len() {
            // nb: can't use uint::range() here because result_set grows
            let r = result_set[result_index];

            debug!("result_index=%u, r=%?", result_index, r);

            let mut undo_index = snapshot;
            while undo_index < undo_len {
                // nb: can't use uint::range() here as we move result_set
                let regs = match self.undo_log[undo_index] {
                    AddConstraint(ConstrainVarSubVar(ref a, ref b)) => {
                        Some((re_infer(ReVar(*a)),
                              re_infer(ReVar(*b))))
                    }
                    AddConstraint(ConstrainRegSubVar(ref a, ref b)) => {
                        Some((*a, re_infer(ReVar(*b))))
                    }
                    AddConstraint(ConstrainVarSubReg(ref a, ref b)) => {
                        Some((re_infer(ReVar(*a)), *b))
                    }
                    AddConstraint(ConstrainRegSubReg(a, b)) => {
                        Some((a, b))
                    }
                    _ => {
                        None
                    }
                };

                match regs {
                    None => {}
                    Some((r1, r2)) => {
                        result_set =
                            consider_adding_edge(result_set, r, r1, r2);
                        result_set =
                            consider_adding_edge(result_set, r, r2, r1);
                    }
                }

                undo_index += 1;
            }

            result_index += 1;
        }

        return result_set;

        fn consider_adding_edge(result_set: ~[Region],
                                r: Region,
                                r1: Region,
                                r2: Region) -> ~[Region]
        {
            let mut result_set = result_set;
            if r == r1 { // Clearly, this is potentially inefficient.
                if !result_set.iter().any_(|x| x == r2) {
                    result_set.push(r2);
                }
            }
            return result_set;
        }
    }

    /**
    This function performs the actual region resolution.  It must be
    called after all constraints have been added.  It performs a
    fixed-point iteration to find region values which satisfy all
    constraints, assuming such values can be found; if they cannot,
    errors are reported.
    */
    pub fn resolve_regions(&mut self) -> OptVec<RegionResolutionError> {
        debug!("RegionVarBindings: resolve_regions()");
        let mut errors = opt_vec::Empty;
        let v = self.infer_variable_values(&mut errors);
        self.values.put_back(v);
        errors
    }
}

impl RegionVarBindings {
    fn is_subregion_of(&self, sub: Region, sup: Region) -> bool {
        let rm = self.tcx.region_maps;
        rm.is_subregion_of(sub, sup)
    }

    fn lub_concrete_regions(&self, a: Region, b: Region) -> Region {
        match (a, b) {
          (re_static, _) | (_, re_static) => {
            re_static // nothing lives longer than static
          }

          (re_empty, r) | (r, re_empty) => {
            r // everything lives longer than empty
          }

          (re_infer(ReVar(v_id)), _) | (_, re_infer(ReVar(v_id))) => {
            self.tcx.sess.span_bug(
                self.var_origins[v_id.to_uint()].span(),
                fmt!("lub_concrete_regions invoked with \
                      non-concrete regions: %?, %?", a, b));
          }

          (f @ re_free(ref fr), re_scope(s_id)) |
          (re_scope(s_id), f @ re_free(ref fr)) => {
            // A "free" region can be interpreted as "some region
            // at least as big as the block fr.scope_id".  So, we can
            // reasonably compare free regions and scopes:
            let rm = self.tcx.region_maps;
            match rm.nearest_common_ancestor(fr.scope_id, s_id) {
              // if the free region's scope `fr.scope_id` is bigger than
              // the scope region `s_id`, then the LUB is the free
              // region itself:
              Some(r_id) if r_id == fr.scope_id => f,

              // otherwise, we don't know what the free region is,
              // so we must conservatively say the LUB is static:
              _ => re_static
            }
          }

          (re_scope(a_id), re_scope(b_id)) => {
            // The region corresponding to an outer block is a
            // subtype of the region corresponding to an inner
            // block.
            let rm = self.tcx.region_maps;
            match rm.nearest_common_ancestor(a_id, b_id) {
              Some(r_id) => re_scope(r_id),
              _ => re_static
            }
          }

          (re_free(ref a_fr), re_free(ref b_fr)) => {
             self.lub_free_regions(a_fr, b_fr)
          }

          // For these types, we cannot define any additional
          // relationship:
          (re_infer(ReSkolemized(*)), _) |
          (_, re_infer(ReSkolemized(*))) |
          (re_bound(_), re_bound(_)) |
          (re_bound(_), re_free(_)) |
          (re_bound(_), re_scope(_)) |
          (re_free(_), re_bound(_)) |
          (re_scope(_), re_bound(_)) => {
            if a == b {a} else {re_static}
          }
        }
    }

    fn lub_free_regions(&self,
                        a: &FreeRegion,
                        b: &FreeRegion) -> ty::Region
    {
        /*!
         * Computes a region that encloses both free region arguments.
         * Guarantee that if the same two regions are given as argument,
         * in any order, a consistent result is returned.
         */

        return match a.cmp(b) {
            Less => helper(self, a, b),
            Greater => helper(self, b, a),
            Equal => ty::re_free(*a)
        };

        fn helper(this: &RegionVarBindings,
                  a: &FreeRegion,
                  b: &FreeRegion) -> ty::Region
        {
            let rm = this.tcx.region_maps;
            if rm.sub_free_region(*a, *b) {
                ty::re_free(*b)
            } else if rm.sub_free_region(*b, *a) {
                ty::re_free(*a)
            } else {
                ty::re_static
            }
        }
    }

    fn glb_concrete_regions(&self,
                            a: Region,
                            b: Region)
                         -> cres<Region> {
        debug!("glb_concrete_regions(%?, %?)", a, b);
        match (a, b) {
            (re_static, r) | (r, re_static) => {
                // static lives longer than everything else
                Ok(r)
            }

            (re_empty, _) | (_, re_empty) => {
                // nothing lives shorter than everything else
                Ok(re_empty)
            }

            (re_infer(ReVar(v_id)), _) |
            (_, re_infer(ReVar(v_id))) => {
                self.tcx.sess.span_bug(
                    self.var_origins[v_id.to_uint()].span(),
                    fmt!("glb_concrete_regions invoked with \
                          non-concrete regions: %?, %?", a, b));
            }

            (re_free(ref fr), s @ re_scope(s_id)) |
            (s @ re_scope(s_id), re_free(ref fr)) => {
                // Free region is something "at least as big as
                // `fr.scope_id`."  If we find that the scope `fr.scope_id` is bigger
                // than the scope `s_id`, then we can say that the GLB
                // is the scope `s_id`.  Otherwise, as we do not know
                // big the free region is precisely, the GLB is undefined.
                let rm = self.tcx.region_maps;
                match rm.nearest_common_ancestor(fr.scope_id, s_id) {
                    Some(r_id) if r_id == fr.scope_id => Ok(s),
                    _ => Err(ty::terr_regions_no_overlap(b, a))
                }
            }

            (re_scope(a_id), re_scope(b_id)) => {
                self.intersect_scopes(a, b, a_id, b_id)
            }

            (re_free(ref a_fr), re_free(ref b_fr)) => {
                self.glb_free_regions(a_fr, b_fr)
            }

            // For these types, we cannot define any additional
            // relationship:
            (re_infer(ReSkolemized(*)), _) |
            (_, re_infer(ReSkolemized(*))) |
            (re_bound(_), re_bound(_)) |
            (re_bound(_), re_free(_)) |
            (re_bound(_), re_scope(_)) |
            (re_free(_), re_bound(_)) |
            (re_scope(_), re_bound(_)) => {
                if a == b {
                    Ok(a)
                } else {
                    Err(ty::terr_regions_no_overlap(b, a))
                }
            }
        }
    }

    fn glb_free_regions(&self,
                        a: &FreeRegion,
                        b: &FreeRegion) -> cres<ty::Region>
    {
        /*!
         * Computes a region that is enclosed by both free region arguments,
         * if any. Guarantees that if the same two regions are given as argument,
         * in any order, a consistent result is returned.
         */

        return match a.cmp(b) {
            Less => helper(self, a, b),
            Greater => helper(self, b, a),
            Equal => Ok(ty::re_free(*a))
        };

        fn helper(this: &RegionVarBindings,
                  a: &FreeRegion,
                  b: &FreeRegion) -> cres<ty::Region>
        {
            let rm = this.tcx.region_maps;
            if rm.sub_free_region(*a, *b) {
                Ok(ty::re_free(*a))
            } else if rm.sub_free_region(*b, *a) {
                Ok(ty::re_free(*b))
            } else {
                this.intersect_scopes(ty::re_free(*a), ty::re_free(*b),
                                      a.scope_id, b.scope_id)
            }
        }
    }

    fn report_type_error(&mut self,
                         origin: SubregionOrigin,
                         terr: &ty::type_err) {
        let terr_str = ty::type_err_to_str(self.tcx, terr);
        self.tcx.sess.span_err(origin.span(), terr_str);
    }

    fn intersect_scopes(&self,
                        region_a: ty::Region,
                        region_b: ty::Region,
                        scope_a: ast::node_id,
                        scope_b: ast::node_id) -> cres<Region>
    {
        // We want to generate the intersection of two
        // scopes or two free regions.  So, if one of
        // these scopes is a subscope of the other, return
        // it.  Otherwise fail.
        debug!("intersect_scopes(scope_a=%?, scope_b=%?, region_a=%?, region_b=%?)",
               scope_a, scope_b, region_a, region_b);
        let rm = self.tcx.region_maps;
        match rm.nearest_common_ancestor(scope_a, scope_b) {
            Some(r_id) if scope_a == r_id => Ok(re_scope(scope_b)),
            Some(r_id) if scope_b == r_id => Ok(re_scope(scope_a)),
            _ => Err(ty::terr_regions_no_overlap(region_a, region_b))
        }
    }
}

// ______________________________________________________________________

#[deriving(Eq)]
enum Direction { Incoming = 0, Outgoing = 1 }

#[deriving(Eq)]
enum Classification { Expanding, Contracting }

enum GraphNodeValue { NoValue, Value(Region), ErrorValue }

struct GraphNode {
    origin: RegionVariableOrigin,
    classification: Classification,
    value: GraphNodeValue,
    head_edge: [uint, ..2],
}

struct GraphEdge {
    next_edge: [uint, ..2],
    constraint: Constraint,
}

struct Graph {
    nodes: ~[GraphNode],
    edges: ~[GraphEdge],
}

struct RegionAndOrigin {
    region: Region,
    origin: SubregionOrigin,
}

impl RegionVarBindings {
    fn infer_variable_values(&mut self,
                             errors: &mut OptVec<RegionResolutionError>)
                             -> ~[GraphNodeValue] {
        let mut graph = self.construct_graph();
        self.expansion(&mut graph);
        self.contraction(&mut graph);
        self.collect_concrete_region_errors(&graph, errors);
        self.extract_values_and_collect_conflicts(&graph, errors)
    }

    fn construct_graph(&mut self) -> Graph {
        let num_vars = self.num_vars();
        let num_edges = self.constraints.len();

        let nodes = vec::from_fn(num_vars, |var_idx| {
            GraphNode {
                // All nodes are initially classified as contracting; during
                // the expansion phase, we will shift the classification for
                // those nodes that have a concrete region predecessor to
                // Expanding.
                classification: Contracting,
                origin: self.var_origins[var_idx],
                value: NoValue,
                head_edge: [uint::max_value, uint::max_value]
            }
        });

        // It would be nice to write this using map():
        let mut edges = vec::with_capacity(num_edges);
        for self.constraints.iter().advance |(constraint, _)| {
            edges.push(GraphEdge {
                next_edge: [uint::max_value, uint::max_value],
                constraint: *constraint,
            });
        }

        let mut graph = Graph {
            nodes: nodes,
            edges: edges
        };

        for uint::range(0, num_edges) |edge_idx| {
            match graph.edges[edge_idx].constraint {
              ConstrainVarSubVar(a_id, b_id) => {
                insert_edge(&mut graph, a_id, Outgoing, edge_idx);
                insert_edge(&mut graph, b_id, Incoming, edge_idx);
              }
              ConstrainRegSubVar(_, b_id) => {
                insert_edge(&mut graph, b_id, Incoming, edge_idx);
              }
              ConstrainVarSubReg(a_id, _) => {
                insert_edge(&mut graph, a_id, Outgoing, edge_idx);
              }
              ConstrainRegSubReg(*) => {
                  // Relations between two concrete regions do not
                  // require an edge in the graph.
              }
            }
        }

        return (graph);

        fn insert_edge(graph: &mut Graph,
                       node_id: RegionVid,
                       edge_dir: Direction,
                       edge_idx: uint) {
            //! Insert edge `edge_idx` on the link list of edges in direction
            //! `edge_dir` for the node `node_id`
            let edge_dir = edge_dir as uint;
            assert_eq!(graph.edges[edge_idx].next_edge[edge_dir],
                       uint::max_value);
            let n = node_id.to_uint();
            let prev_head = graph.nodes[n].head_edge[edge_dir];
            graph.edges[edge_idx].next_edge[edge_dir] = prev_head;
            graph.nodes[n].head_edge[edge_dir] = edge_idx;
        }
    }

    fn expansion(&mut self, graph: &mut Graph) {
        do iterate_until_fixed_point(~"Expansion", graph) |nodes, edge| {
            match edge.constraint {
              ConstrainRegSubVar(a_region, b_vid) => {
                let b_node = &mut nodes[b_vid.to_uint()];
                self.expand_node(a_region, b_vid, b_node)
              }
              ConstrainVarSubVar(a_vid, b_vid) => {
                match nodes[a_vid.to_uint()].value {
                  NoValue | ErrorValue => false,
                  Value(a_region) => {
                    let b_node = &mut nodes[b_vid.to_uint()];
                    self.expand_node(a_region, b_vid, b_node)
                  }
                }
              }
              ConstrainVarSubReg(*) => {
                // This is a contraction constraint.  Ignore it.
                false
              }
              ConstrainRegSubReg(*) => {
                // No region variables involved. Ignore.
                false
              }
            }
        }
    }

    fn expand_node(&mut self,
                   a_region: Region,
                   b_vid: RegionVid,
                   b_node: &mut GraphNode)
                   -> bool {
        debug!("expand_node(%?, %? == %?)",
               a_region, b_vid, b_node.value);

        b_node.classification = Expanding;
        match b_node.value {
          NoValue => {
            debug!("Setting initial value of %? to %?", b_vid, a_region);

            b_node.value = Value(a_region);
            return true;
          }

          Value(cur_region) => {
            let lub = self.lub_concrete_regions(a_region, cur_region);
            if lub == cur_region {
                return false;
            }

            debug!("Expanding value of %? from %? to %?",
                   b_vid, cur_region, lub);

            b_node.value = Value(lub);
            return true;
          }

          ErrorValue => {
            return false;
          }
        }
    }

    fn contraction(&mut self,
                   graph: &mut Graph) {
        do iterate_until_fixed_point(~"Contraction", graph) |nodes, edge| {
            match edge.constraint {
              ConstrainRegSubVar(*) => {
                // This is an expansion constraint.  Ignore.
                false
              }
              ConstrainVarSubVar(a_vid, b_vid) => {
                match nodes[b_vid.to_uint()].value {
                  NoValue | ErrorValue => false,
                  Value(b_region) => {
                    let a_node = &mut nodes[a_vid.to_uint()];
                    self.contract_node(a_vid, a_node, b_region)
                  }
                }
              }
              ConstrainVarSubReg(a_vid, b_region) => {
                let a_node = &mut nodes[a_vid.to_uint()];
                self.contract_node(a_vid, a_node, b_region)
              }
              ConstrainRegSubReg(*) => {
                // No region variables involved. Ignore.
                false
              }
            }
        }
    }

    fn contract_node(&mut self,
                     a_vid: RegionVid,
                     a_node: &mut GraphNode,
                     b_region: Region)
                     -> bool {
        debug!("contract_node(%? == %?/%?, %?)",
               a_vid, a_node.value, a_node.classification, b_region);

        return match a_node.value {
            NoValue => {
                assert_eq!(a_node.classification, Contracting);
                a_node.value = Value(b_region);
                true // changed
            }

            ErrorValue => {
                false // no change
            }

            Value(a_region) => {
                match a_node.classification {
                    Expanding => {
                        check_node(self, a_vid, a_node, a_region, b_region)
                    }
                    Contracting => {
                        adjust_node(self, a_vid, a_node, a_region, b_region)
                    }
                }
            }
        };

        fn check_node(this: &mut RegionVarBindings,
                      a_vid: RegionVid,
                      a_node: &mut GraphNode,
                      a_region: Region,
                      b_region: Region)
                   -> bool {
            if !this.is_subregion_of(a_region, b_region) {
                debug!("Setting %? to ErrorValue: %? not subregion of %?",
                       a_vid, a_region, b_region);
                a_node.value = ErrorValue;
            }
            false
        }

        fn adjust_node(this: &mut RegionVarBindings,
                       a_vid: RegionVid,
                       a_node: &mut GraphNode,
                       a_region: Region,
                       b_region: Region)
                    -> bool {
            match this.glb_concrete_regions(a_region, b_region) {
                Ok(glb) => {
                    if glb == a_region {
                        false
                    } else {
                        debug!("Contracting value of %? from %? to %?",
                               a_vid, a_region, glb);
                        a_node.value = Value(glb);
                        true
                    }
                }
                Err(_) => {
                    debug!("Setting %? to ErrorValue: no glb of %?, %?",
                           a_vid, a_region, b_region);
                    a_node.value = ErrorValue;
                    false
                }
            }
        }
    }

    fn collect_concrete_region_errors(
        &mut self,
        graph: &Graph,
        errors: &mut OptVec<RegionResolutionError>)
    {
        let num_edges = graph.edges.len();
        for uint::range(0, num_edges) |edge_idx| {
            let edge = &graph.edges[edge_idx];
            let origin = self.constraints.get_copy(&edge.constraint);

            let (sub, sup) = match edge.constraint {
                ConstrainVarSubVar(*) |
                ConstrainRegSubVar(*) |
                ConstrainVarSubReg(*) => {
                    loop;
                }
                ConstrainRegSubReg(sub, sup) => {
                    (sub, sup)
                }
            };

            if self.is_subregion_of(sub, sup) {
                loop;
            }

            debug!("ConcreteFailure: !(sub <= sup): sub=%?, sup=%?",
                   sub, sup);
            errors.push(ConcreteFailure(origin, sub, sup));
        }
    }

    fn extract_values_and_collect_conflicts(
        &mut self,
        graph: &Graph,
        errors: &mut OptVec<RegionResolutionError>)
        -> ~[GraphNodeValue]
    {
        debug!("extract_values_and_collect_conflicts()");

        // This is the best way that I have found to suppress
        // duplicate and related errors. Basically we keep a set of
        // flags for every node. Whenever an error occurs, we will
        // walk some portion of the graph looking to find pairs of
        // conflicting regions to report to the user. As we walk, we
        // trip the flags from false to true, and if we find that
        // we've already reported an error involving any particular
        // node we just stop and don't report the current error.  The
        // idea is to report errors that derive from independent
        // regions of the graph, but not those that derive from
        // overlapping locations.
        let mut dup_vec = graph.nodes.map(|_| uint::max_value);

        graph.nodes.iter().enumerate().transform(|(idx, node)| {
            match node.value {
                Value(_) => {
                    /* Inference successful */
                }
                NoValue => {
                    /* Unconstrained inference: do not report an error
                       until the value of this variable is requested.
                       After all, sometimes we make region variables but never
                       really use their values. */
                }
                ErrorValue => {
                    /* Inference impossible, this value contains
                       inconsistent constraints.

                       I think that in this case we should report an
                       error now---unlike the case above, we can't
                       wait to see whether the user needs the result
                       of this variable.  The reason is that the mere
                       existence of this variable implies that the
                       region graph is inconsistent, whether or not it
                       is used.

                       For example, we may have created a region
                       variable that is the GLB of two other regions
                       which do not have a GLB.  Even if that variable
                       is not used, it implies that those two regions
                       *should* have a GLB.

                       At least I think this is true. It may be that
                       the mere existence of a conflict in a region variable
                       that is not used is not a problem, so if this rule
                       starts to create problems we'll have to revisit
                       this portion of the code and think hard about it. =) */

                    let node_vid = RegionVid { id: idx };
                    match node.classification {
                        Expanding => {
                            self.collect_error_for_expanding_node(
                                graph, dup_vec, node_vid, errors);
                        }
                        Contracting => {
                            self.collect_error_for_contracting_node(
                                graph, dup_vec, node_vid, errors);
                        }
                    }
                }
            }

            node.value
        }).collect()
    }

    fn collect_error_for_expanding_node(
        &mut self,
        graph: &Graph,
        dup_vec: &mut [uint],
        node_idx: RegionVid,
        errors: &mut OptVec<RegionResolutionError>)
    {
        // Errors in expanding nodes result from a lower-bound that is
        // not contained by an upper-bound.
        let (lower_bounds, lower_dup) =
            self.collect_concrete_regions(graph, node_idx, Incoming, dup_vec);
        let (upper_bounds, upper_dup) =
            self.collect_concrete_regions(graph, node_idx, Outgoing, dup_vec);

        if lower_dup || upper_dup {
            return;
        }

        for lower_bounds.iter().advance |lower_bound| {
            for upper_bounds.iter().advance |upper_bound| {
                if !self.is_subregion_of(lower_bound.region,
                                         upper_bound.region) {
                    errors.push(SubSupConflict(
                        self.var_origins[node_idx.to_uint()],
                        lower_bound.origin,
                        lower_bound.region,
                        upper_bound.origin,
                        upper_bound.region));
                    return;
                }
            }
        }

        self.tcx.sess.span_bug(
            self.var_origins[node_idx.to_uint()].span(),
            fmt!("collect_error_for_expanding_node() could not find error \
                  for var %?, lower_bounds=%s, upper_bounds=%s",
                 node_idx,
                 lower_bounds.map(|x| x.region).repr(self.tcx),
                 upper_bounds.map(|x| x.region).repr(self.tcx)));
    }

    fn collect_error_for_contracting_node(
        &mut self,
        graph: &Graph,
        dup_vec: &mut [uint],
        node_idx: RegionVid,
        errors: &mut OptVec<RegionResolutionError>)
    {
        // Errors in contracting nodes result from two upper-bounds
        // that have no intersection.
        let (upper_bounds, dup_found) =
            self.collect_concrete_regions(graph, node_idx, Outgoing, dup_vec);

        if dup_found {
            return;
        }

        for upper_bounds.iter().advance |upper_bound_1| {
            for upper_bounds.iter().advance |upper_bound_2| {
                match self.glb_concrete_regions(upper_bound_1.region,
                                                upper_bound_2.region) {
                  Ok(_) => {}
                  Err(_) => {
                    errors.push(SupSupConflict(
                        self.var_origins[node_idx.to_uint()],
                        upper_bound_1.origin,
                        upper_bound_1.region,
                        upper_bound_2.origin,
                        upper_bound_2.region));
                    return;
                  }
                }
            }
        }

        self.tcx.sess.span_bug(
            self.var_origins[node_idx.to_uint()].span(),
            fmt!("collect_error_for_contracting_node() could not find error \
                  for var %?, upper_bounds=%s",
                 node_idx,
                 upper_bounds.map(|x| x.region).repr(self.tcx)));
    }

    fn collect_concrete_regions(&mut self,
                                graph: &Graph,
                                orig_node_idx: RegionVid,
                                dir: Direction,
                                dup_vec: &mut [uint])
                                -> (~[RegionAndOrigin], bool) {
        struct WalkState {
            set: HashSet<RegionVid>,
            stack: ~[RegionVid],
            result: ~[RegionAndOrigin],
            dup_found: bool
        }
        let mut state = WalkState {
            set: HashSet::new(),
            stack: ~[orig_node_idx],
            result: ~[],
            dup_found: false
        };
        state.set.insert(orig_node_idx);

        // to start off the process, walk the source node in the
        // direction specified
        process_edges(self, &mut state, graph, orig_node_idx, dir);

        while !state.stack.is_empty() {
            let node_idx = state.stack.pop();
            let classification = graph.nodes[node_idx.to_uint()].classification;

            // check whether we've visited this node on some previous walk
            if dup_vec[node_idx.to_uint()] == uint::max_value {
                dup_vec[node_idx.to_uint()] = orig_node_idx.to_uint();
            } else if dup_vec[node_idx.to_uint()] != orig_node_idx.to_uint() {
                state.dup_found = true;
            }

            debug!("collect_concrete_regions(orig_node_idx=%?, node_idx=%?, \
                    classification=%?)",
                   orig_node_idx, node_idx, classification);

            // figure out the direction from which this node takes its
            // values, and search for concrete regions etc in that direction
            let dir = match classification {
                Expanding => Incoming,
                Contracting => Outgoing
            };

            process_edges(self, &mut state, graph, node_idx, dir);
        }

        let WalkState {result, dup_found, _} = state;
        return (result, dup_found);

        fn process_edges(this: &mut RegionVarBindings,
                         state: &mut WalkState,
                         graph: &Graph,
                         source_vid: RegionVid,
                         dir: Direction) {
            debug!("process_edges(source_vid=%?, dir=%?)", source_vid, dir);

            for this.each_edge(graph, source_vid, dir) |edge| {
                match edge.constraint {
                    ConstrainVarSubVar(from_vid, to_vid) => {
                        let opp_vid =
                            if from_vid == source_vid {to_vid} else {from_vid};
                        if state.set.insert(opp_vid) {
                            state.stack.push(opp_vid);
                        }
                    }

                    ConstrainRegSubVar(region, _) |
                    ConstrainVarSubReg(_, region) => {
                        state.result.push(RegionAndOrigin {
                            region: region,
                            origin: this.constraints.get_copy(&edge.constraint)
                        });
                    }

                    ConstrainRegSubReg(*) => {}
                }
            }
        }
    }

    pub fn each_edge(&self,
                     graph: &Graph,
                     node_idx: RegionVid,
                     dir: Direction,
                     op: &fn(edge: &GraphEdge) -> bool)
                     -> bool {
        let mut edge_idx =
            graph.nodes[node_idx.to_uint()].head_edge[dir as uint];
        while edge_idx != uint::max_value {
            let edge_ptr = &graph.edges[edge_idx];
            if !op(edge_ptr) {
                return false;
            }
            edge_idx = edge_ptr.next_edge[dir as uint];
        }
        return true;
    }
}

fn iterate_until_fixed_point(
    tag: ~str,
    graph: &mut Graph,
    body: &fn(nodes: &mut [GraphNode], edge: &GraphEdge) -> bool)
{
    let mut iteration = 0;
    let mut changed = true;
    let num_edges = graph.edges.len();
    while changed {
        changed = false;
        iteration += 1;
        debug!("---- %s Iteration #%u", tag, iteration);
        for uint::range(0, num_edges) |edge_idx| {
            changed |= body(graph.nodes, &graph.edges[edge_idx]);
            debug!(" >> Change after edge #%?: %?",
                   edge_idx, graph.edges[edge_idx]);
        }
    }
    debug!("---- %s Complete after %u iteration(s)", tag, iteration);
}
