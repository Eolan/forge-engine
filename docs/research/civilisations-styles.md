# Research — Civilisations and styles: vernacular form, style grammars, decor, materials by mathematics, the games' culture sets

> Companion to `city-generation.md` (roads, lots, buildings as grammar-driven kit assemblies, the
> D-039 style set on a 0.5 m module grid), to `vegetation-materials.md` §4–§5 (trim sheets, the
> regional material set, unified materials), to `procedural.md` §4–§5 (settlements, grammars, the
> texturing canon) and to `planet-environment.md` (the baked climate atlas and its biomes: the
> climate a civilisation answers to). Written 2026-09-26 for the owner's brief — mathematical and
> procedural techniques for decors, objects, materials, textures and architectures of different
> styles, for different civilisations and cultures, from primitive to medieval to current to
> futuristic — with the ideas #81–#91 in view (#84 the living city, #86 modular buildings, #87
> street dressing, #88 interiors, #91 an organic town, #80 the bases in the belt). Every citation
> was checked that day against a reachable page or, where the network proxy refused the host,
> against the search engine's record of it; the distinction is kept per entry ("verified through
> search results", for issue #99) and summarised under [Verification notes](#verification-notes).
> What was looked for and not found is under [Checked and left out](#checked-and-left-out). Entries
> the sibling files already carry are referenced by section and not repeated.

The question is what the published work says about *why* buildings, objects and surfaces look the
way they do in a given place and time, and how much of that can be written as parameters and rules
so that one generator, fed a culture, an era and a climate, produces a hut, a Provençal village, a
downtown block or a space habitat without an art team drawing each. The short answer: the
vernacular literature already reads form as a function of climate, materials and culture, in a
form a generator can consume (orientation, wall mass, opening ratio, roof pitch, courtyard or
stilts, compact or spread); fifty years of shape grammars prove that a *style* is a finite rule set
with attributes, so a culture and an era are data over a shared split grammar exactly as D-039
proposes; the texturing canon plus the weathering and by-example work make a culture's *materials*
a palette derived from its resources and its wear; and every game that ships many civilisations
does the same two things — a handful of hand-set *sets* per culture, and generation only *within*
a set. Forge's `Civilisation` is therefore a D-035 package layer that selects and patches style
sets, material sets, prop sets and grammar attributes, with the climate response read from the
atlas rather than authored, and the one real risk — every culture looking like the same generator
in a different colour — is fought with asymmetry, landmarks, hand-authored overrides and wear.

> **State of the art in five sentences.** Vernacular form is explained, not invented: Rapoport's
> *House Form and Culture* puts culture first and climate, materials and technology as modifying
> factors, Oliver's three-volume encyclopedia catalogues the result by region with typologies,
> environment and materials as the organising concepts, and the bioclimatic school (Olgyay's four
> climatic regions, Givoni's chart, the Mahoney tables) turns monthly temperature and humidity into
> design recommendations — orientation, wall mass, opening size, shading, roof — that are already
> parameters. A style is a grammar: from Stiny & Mitchell's Palladian villas (1978) through Wright's
> Prairie houses, Flemming's Queen Anne, Duarte's Malagueira, Li's Yingzao Fashi and Kaplan's star
> patterns to the CGA reconstructions of Pompeii and Puuc Maya buildings, every published style
> grammar is a small set of split and placement rules plus attributes, which is what D-039's
> control grammar already separates from the shared split grammar. Objects and decor follow the
> same pattern at a smaller scale — Infinigen Indoors ships 79 procedural generators and a
> constraint language for their arrangement, ShapeAssembly writes furniture as cuboid programs,
> Merrell's and Yu's layout optimisers place them from design guidelines — while studios dress rooms
> and streets from kits under an art director's brief. Materials are mathematics (Perlin, Worley,
> fBm, domain warping, reaction–diffusion, Gabor and phasor noise, glint BRDFs) or examples
> (Efros–Leung to PatchMatch, Wang tiles, histogram-preserving blending and hex-tiling, which Forge
> has), aged by flow, patina, γ-ton tracing and appearance manifolds, and the neural work
> (MaterialGAN, neural BTFs, MatFormer) is a reference for what a graph can express rather than a
> dependency. The shipped games converge on eleven to a few dozen hand-set architecture sets per
> culture with era progression inside each, cultures and materials as data files where the game is
> data-driven (Dwarf Fortress, RimWorld, Minecraft's structure templates), and procedural variation
> only inside a set — No Man's Sky's ships are archetypes per race with generated parts, Starfield's
> cities are hand-built from Bethesda's kits.

**Contents**

1. [Why buildings look the way they do: climate, materials and culture as parameters](#1-why-buildings-look-the-way-they-do-climate-materials-and-culture-as-parameters)
2. [Style as a grammar, per culture and era](#2-style-as-a-grammar-per-culture-and-era)
3. [Decor, props and objects](#3-decor-props-and-objects)
4. [Materials and textures by mathematics](#4-materials-and-textures-by-mathematics)
5. [What the shipped games do with civilisations](#5-what-the-shipped-games-do-with-civilisations)
6. [How it all fits Forge](#6-how-it-all-fits-forge)
7. [Recommendation for Forge](#recommendation-for-forge)
8. [What the numbers say](#what-the-numbers-say)
9. [Checked and left out](#checked-and-left-out)
10. [Verification notes](#verification-notes)

---

## 1. Why buildings look the way they do: climate, materials and culture as parameters

The generator needs a theory of form before it needs rules, or the rules are guesses. The
vernacular literature supplies one, and its authors disagree usefully: the bioclimatic school reads
form from climate, Rapoport reads it from culture with climate as a constraint, and Oliver
catalogues what people actually built. Climate sets the *range* of forms that work, resources set
the *materials*, culture picks *within* the range and adds the meaning.

**Amos Rapoport. *House Form and Culture*. Prentice-Hall (Foundations of Cultural Geography),
Englewood Cliffs, 1969, 150 pp.** [book] [foundational]
<https://openlibrary.org/books/OL5683075M/House_form_and_culture.> (verified through search
results)

The argument every later study answers: the catalogue's description says the book "brings order to
the complex variety of dwelling forms worldwide by concentrating on the forces that have shaped the
dwelling", and its thesis is that socio-cultural forces are primary while climate, materials and
technology are modifying factors — the same climate produces the courtyard house and the tent.
*Bearing:* the record's order of precedence. A `Civilisation` carries the culture's choices
(courtyard or not, the sacred building's place, privacy gradients) as explicit fields, and the
climate response only *defaults* them from the atlas. Rapoport is the argument against deriving
everything from temperature and rainfall, which would make every hot-dry culture the same.

**Paul Oliver (ed.). *Encyclopedia of Vernacular Architecture of the World*. Cambridge University
Press, 1997, 3 vols., 2 500 pp.; and Oliver, *Dwellings: The Vernacular House Worldwide*, Phaidon,
2003 (first edition 1987).** [book] [foundational] [still-current]
<https://www.cambridge.org/us/universitypress/subjects/arts-theatre-culture/architecture/encyclopedia-vernacular-architecture-world>
· <https://searchworks.stanford.edu/view/5421658> (verified through search results)

The reference catalogue: volume 1 "discusses broad concepts such as 'typologies', 'symbolism and
decoration', 'environment' and 'materials and building resources'", volumes 2 and 3 survey the
world by continent and region with "contributions from researchers from 80 countries". *Dwellings*
is the one-volume argument: vernacular building is "in excess of 90 per cent of the world's
buildings, including some 800 million dwellings", built by people who are "self-built by their
owner-occupiers or built by members of a community who share cultural values and norms".
*Bearing:* the field guide for authoring a culture's parameters honestly — for each region a page
gives plan type, roof form, materials, decoration and settlement pattern, which is the column set
of the `Civilisation` record — and the proportion that is the design brief: the hero buildings are
the exception and the vernacular is the ground, so a style set is mostly dwellings plus a handful
of landmark kinds. A mixed or invented culture is an interpolation between two entries.

**Hassan Fathy. *Natural Energy and Vernacular Architecture: Principles and Examples with Reference
to Hot Arid Climates*. University of Chicago Press for the United Nations University, 1986.**
[book] [foundational]
<https://openlibrary.org/books/OL22214100M/Natural_energy_and_vernacular_architecture> (verified
through search results)

The hot-dry handbook by the architect of New Gourna: research on climate control in the Middle East
"to demonstrate advantages of locally available building materials and traditional building
methods" — the courtyard as a cold-air reservoir, the *malqaf* wind catcher, the *mashrabiya*
screen, domes and vaults in mud brick, thick walls with a long thermal lag.
*Bearing:* the parameter set of the hot-dry response: courtyard ratio, wall thickness (0.4–0.6 m,
one module step), opening ratio under 10 % on the street side, a wind-catcher module on the roof,
domes and barrel vaults as roof modules. Each is one attribute of the control grammar; the wind
catcher and dome are modules the futuristic set does not need and the primitive set has in a cruder
form.

**Victor Olgyay. *Design with Climate: Bioclimatic Approach to Architectural Regionalism*.
Princeton University Press, 1963; new and expanded edition 2015.** [book] [foundational]
[still-current]
<https://press.princeton.edu/books/paperback/9780691169736/design-with-climate> (verified through
search results)

The bioclimatic method: the book "explores the impact of climate on shelter design, identifying
four distinct climatic regions and explaining the effect of each on orientation, air movement,
site, and materials", with the chart that maps temperature and humidity onto comfort and the
measures that restore it.
*Bearing:* four regions — cool, temperate, hot-arid, hot-humid — are the first-level switch of the
climate response, and Olgyay's per-region tables (orientation, plan elongation, surface-to-volume
ratio) are the defaults for `SettlementForm` and the mass model: compact and elongated east–west in
the cold, open and spread in the hot-humid, inward and dense in the hot-arid.

**Baruch Givoni. *Man, Climate and Architecture*. Elsevier (Architectural Science Series), 1969;
second edition 1976.** [book] [foundational]
<https://openlibrary.org/books/OL14563551M/Man_climate_and_architecture> (verified through search
results)

The building bioclimatic chart in its usual form: from outdoor monthly temperature and humidity it
"predicts comfort conditions within the building" and marks the zones where mass, night
ventilation, evaporative cooling or heating are the right strategies.
*Bearing:* a lookup Forge can bake: D-034's atlas stores monthly temperature, humidity and diurnal
range per cell, so `climate_response(cell)` is twelve lookups into Givoni's zones and a vote — a
wall-mass, ventilation and shading class per settlement without an author, the move
planet-environment.md made with Köppen for biomes.

**Otto H. Koenigsberger, T. G. Ingersoll, Alan Mayhew, S. V. Szokolay. *Manual of Tropical Housing
and Building, Part 1: Climatic Design*. Longman, 1974 (the Mahoney tables).** [book] [foundational]
<https://en.wikipedia.org/wiki/Mahoney_tables> (verified through search results)

The most mechanical of the climate-to-form methods: the textbook "introduces the 'Mahoney tables'
design aid" and shows "how practical solutions are derived from theoretical principles". The tables
take monthly maxima, minima and humidity, classify each month by day and night, count indicators
(humid or arid, need for air movement, thermal storage, rain protection) and turn the counts into
recommendations: layout orientation, spacing, air movement, opening size as a percentage of wall,
wall and roof mass, rain protection.
*Bearing:* already an algorithm, and the only method here that outputs numbers rather than prose.
It is the first thing to implement in `forge-procgen::civilisation`: an atlas cell in, a
`ClimateResponse` out, under a hundred lines, checked against the tables' worked examples.

**Christopher Alexander, Sara Ishikawa, Murray Silverstein et al. *A Pattern Language: Towns,
Buildings, Construction*. Oxford University Press, 1977.** [book] [foundational]
<https://en.wikipedia.org/wiki/A_Pattern_Language> (verified through search results)

"253 patterns which serve as generic guiding principles for design", each a problem, a discussion
and a solution, and "patterns range in scale from regional planning through to interior design":
positive outdoor space, entrance transition, light on two sides of every room, thick walls, window
place. Two companions were confirmed the same way: DeKay & Brown's *Sun, Wind, and Light* (3rd ed.,
Wiley, 2014), a strategy catalogue for "designing buildings that use the sun for heating, wind for
cooling, and daylight for natural lighting" with the dimensions the patterns lack, and Ching's
*Architecture: Form, Space, and Order* (4th ed., Wiley, 2014), "the classic introduction to the
basic vocabulary of architectural design" — organisations of form, transformations, proportion and
ordering principles.
*Bearing:* a rule catalogue with dependencies is what a settlement and interior grammar is, but
Forge should implement Alexander as a *scorecard* (with the GDMC rubric of `procedural.md` §4) —
does the plaza have edges, do rooms get light from two sides — so #88's interiors get acceptance
criteria; DeKay & Brown supply the numbers a rule needs, and Ching the names the grammar's
operations and attributes should carry (additive and subtractive massing, centralised, linear,
radial, clustered and gridded organisations) so an authored style reads like architecture.

**R. A. Buswell, W. R. Leal de Silva, S. Z. Jones, J. Dirrenberger. "3D printing using concrete
extrusion: A roadmap for research." *Cement and Concrete Research* 112, 2018, 37–49.** [paper]
[recent]
<https://www.sciencedirect.com/science/article/pii/S0008884617311924> (DOI
10.1016/j.cemconres.2018.05.006; verified through search results)

The one material of the owner's list with no vernacular yet: "computer-controlled placement of
extruded cement-based mortar to create physical objects layer-by-layer", with walls printed in situ
and the geometry constrained by the fresh material's ability to bear the next layer.
*Bearing:* the near-future style's honest signature — horizontal striations on curved,
self-supporting, corbelled walls with no formwork corners — is a stripe field along the wall's
isolines plus a mass rule that prefers rounded plans: one tileable and one rule between the modern
and the futuristic sets.

**Climate to form, as the generator will read it.** Distilled from the sources above; the
`ClimateResponse` fields are the columns, and a culture's record overrides any cell.

| Response class | Settlement | Mass and plan | Walls | Openings | Roof | Signature modules |
|---|---|---|---|---|---|---|
| hot-arid | compact, narrow shaded streets, inward | courtyard, 1–2 storeys, thick party walls | heavy earth or stone, 0.5 m, whitewashed or earth-coloured | small, high, screened; < 10 % street side | flat or domed, used at night | wind catcher, screen, dome, courtyard fountain |
| hot-humid | spread, open, along water and breeze | raised on stilts, single depth, open plan | light: timber, bamboo, woven panels | large, shaded, louvred; walls that open | steep (> 40°), wide eaves, thatch or leaf | stilt, veranda, louvre, eave bracket |
| temperate | clustered on slopes and roads | compact rectangle, 2–3 storeys | stone base, timber frame or brick above, plaster | moderate, 15–25 %, shutters | pitched 25–35°, tile or slate | shutter, cornice, chimney, dormer |
| cold | compact, south-facing, wind-sheltered | small surface-to-volume, shared walls, low | thick insulating: log, sod, stone with timber lining | small, few, south | steep for shedding or low with retained snow; deep eaves | porch (air lock), stove, snow guard, storm shutter |

**Materials from resources.** The palette follows the geology, the biome and the era: the
`Civilisation` names its resources (forest, clay, limestone, slate, grassland, coast; steel and
glass; regolith and composites) and §4's last table derives the material set from them;
planet-environment.md's biome table and terrain-genesis.md's `hardness` layers already say which
resources a site has.

---

## 2. Style as a grammar, per culture and era

Shape grammars were invented to describe *styles* — a corpus of buildings by one architect or one
culture — and every published grammar is evidence that a style is finite: a few dozen rules, a
handful of attributes, one derivation per building. Each entry below contributes something D-039's
grammar does not have yet; the section ends with how a style set carries culture and era as data.

**George Stiny, William J. Mitchell. "The Palladian grammar." *Environment and Planning B* 5(1),
1978, 5–18.** [paper] [foundational]
<https://journals.sagepub.com/doi/10.1068/b050005> (DOI 10.1068/b050005; verified through search
results)

The first architectural style grammar: "a parametric shape grammar that generates the ground plans
of Palladio's villas as a definition of the Palladian style", applied to the "Villa Malcontenta" —
a grid of rooms, then walls, entrance, portico and exterior articulation as ordered stages.
*Bearing:* the staged derivation (grid → rooms → walls → openings → ornament) is the template of a
*classical* set, and it shows that a plan-first and a facade-first grammar (`city-generation.md`
§3) are the same machinery with the stages reordered, which the recommendation's grammar must
allow per style.

**Hank Koning, Julie Eizenberg. "The language of the prairie: Frank Lloyd Wright's prairie houses."
*Environment and Planning B* 8(3), 1981, 295–323.** [paper] [foundational]
<https://journals.sagepub.com/doi/10.1068/b080295> (DOI 10.1068/b080295; verified through search
results)

A grammar over "a corpus of eleven houses from the Winslow house to the Robie house", in which
"the establishment of a fireplace is key to the definition of the prairie-style house" and around
it "functionally distinguished Froebelean-type blocks are recursively added and interpenetrated to
form the basic compositions".
*Bearing:* the *additive* massing grammar — a core, then blocks added and interpenetrated — is the
mass model of every spread, low style (Prairie, the hot-humid compound, the modern villa, a research
station) where D-039's box-fitting-to-the-lot is the wrong first move; the core-first rule
generalises to the hut around its hearth and the mosque around its courtyard.

**Ulrich Flemming. "More than the sum of parts: the grammar of Queen Anne houses." *Environment and
Planning B* 14(3), 1987, 323–350.** [paper] [foundational]
<https://journals.sagepub.com/doi/abs/10.1068/b140323> (DOI 10.1068/b140323; verified through
search results)

The picturesque style "which dominated domestic architecture in the United States in the 1880s",
from "Pittsburgh's historic Shadyside district", as "separate grammars for the generation of floor
plans and for the articulation of plans in three dimensions", the second adding bays, towers,
porches and per-storey material changes.
*Bearing:* the proof that asymmetry is a rule set too, and the antidote to the sameness a
symmetric split grammar produces: Flemming's two-grammar split is how Forge adds "picturesque"
attributes — tower probability, porch wrap, per-floor material — to any style without touching its
plan rules.

**Terry W. Knight. "The forty-one steps: the language of Japanese tea-room designs." *Environment
and Planning B* 8(1), 1981, 97–114.** [paper] [foundational]
<https://www.andrew.cmu.edu/course/48-747/subFrames/readings/Knight.theFortyoneSteps.pdf> (the CMU
course copy; verified through search results)

A grammar for the *chashitsu*: tatami mats (about 0.9 × 1.8 m) as the unit, with the *tokonoma*
alcove, the host's mat, the hearth and the entrances placed by rule relative to one another.
*Bearing:* the mat is a *culture's module grid*: Japanese *sukiya* and *machiya* planning is a
0.9 × 1.8 m grid with posts at the corners, so a style set must be able to declare its bay as a
whole number of 0.5 m steps that is not a multiple of 1 m (1.8 m = 3.6 steps is the test case for
the grid rule); and #88's interior grammar should place furniture by cultural rule (the alcove
opposite the entrance) before any optimiser runs.

**José P. Duarte. "Towards the mass customization of housing: the grammar of Siza's houses at
Malagueira." *Environment and Planning B* 32(3), 2005, 347–380.** [paper] [still-current]
<https://journals.sagepub.com/doi/10.1068/b31124> (DOI 10.1068/b31124; verified through search
results)

Siza's patio houses, "a 1200-unit development based on a corpus of thirty-five houses designed
between 1977 and 1996", generated "by the recursive dissection of rectangles locating four
different functional zones (patio, living, services, and sleeping) and the key placement of the
staircase", towards "an interactive computer system for the design of customized mass housing".
*Bearing:* the closest published thing to a modern Mediterranean row-house set, and proof that a
grammar can produce a thousand houses that read as one neighbourhood and no two alike — Malagueira
exists. The patio dissection is the plan-first rule for the hot-arid and Mediterranean sets, and
Duarte's description grammar beside the shape grammar is the pattern for steering a derivation from
a `Lot`'s use and size.

**Andrew I-kang Li. *A shape grammar for teaching the architectural style of the Yingzao fashi*.
PhD thesis, MIT Department of Architecture, 2001.** [paper] [still-current]
<https://dspace.mit.edu/handle/1721.1/8631> (verified through search results)

A grammar for the Song-dynasty building manual written by "Li Jie (d. 1110) and published in
1103": the *cai-fen* module system in which every dimension of a timber hall is a multiple of a
module set by the building's rank, the bracket sets, the bay system and the roof curve; a teaching
tool that aims to "generate all and more than the designs in the language" so students learn "that
style is a human construct".
*Bearing:* East Asian timber architecture is already a modular grammar written down nine centuries
ago — rank → module → every dimension — the strongest case in the literature for D-039's
grid-and-attributes design, and it gives the grammar a `rank` attribute (a building's status scales
its module) that every culture has in some form.

**Craig S. Kaplan, David H. Salesin. "Islamic star patterns in absolute geometry." *ACM Transactions
on Graphics* 23(2), 2004, 97–119; with Kaplan, "Islamic star patterns from polygons in contact",
*Graphics Interface* 2005; and Nader Hamekasi, Faramarz F. Samavati, Ahmad Nasri, "Interactive
modeling of muqarnas", *Computational Aesthetics (CAe) 2011*, 129–136.** [paper] [foundational]
[still-current]
<https://dl.acm.org/doi/10.1145/990002.990003> (DOI 10.1145/990002.990003; author copy
<https://cs.uwaterloo.ca/~csk/publications/Papers/kaplan_2005.pdf>) ·
<https://dl.acm.org/doi/10.1145/2030441.2030469> (DOI 10.1145/2030441.2030469; verified through
search results)

Najm, "a set of tools built on the axioms of absolute geometry for exploring the design space of
Islamic star patterns", using "inflation tilings" as guides and "a parameterized set of motifs that
can be used to fill the many regular polygons that comprise these tilings"; the 2005 paper derives
the patterns from polygons in contact, the construction most reproduce. Muqarnas, the corbelled
vault ornament, are "composed of several basic structures combined in successive layers", and
Hamekasi's workflow "uses floor plans as guidance" to edit them and "automatically generate new
forms".
*Bearing:* the ornament generators of the Islamic-influenced sets — screens, tile fields, doors and
stucco bands as a procedural texture *and* a procedural mesh from one tiling (a culture's
decoration is a list of pattern families and two numbers each, cooked to a trim row and to a screen
module; other tilings give Chinese lattice and knotwork) — and the model for every plan-driven
*transition* module (wall to dome, a capital, a bracket set): a layered assembly of a few cells
cooked once per parameter set into a module the DAG then LODs, geometry rather than a normal map,
which the cluster DAG and the software rasteriser make affordable.

**Pascal Müller, Tijl Vereenooghe, Peter Wonka, Iken Paap, Luc Van Gool. "Procedural 3D
Reconstruction of Puuc Buildings in Xkipché." *VAST 2006*, 139–146; Simon Haegler, Pascal Müller,
Luc Van Gool, "Procedural Modeling for Digital Cultural Heritage", *EURASIP Journal on Image and
Video Processing* 2009, article 852392; Dylla, Frischer, Müller, Ulmer, Haegler, "Rome Reborn
2.0", *CAA 2008*.** [paper] [still-current]
<https://diglib.eg.org/items/3772fd9d-1072-4d9b-8cf5-ebbc31c4e1a8> (DOI
10.2312/VAST/VAST06/139-146) · <https://link.springer.com/article/10.1155/2009/852392> (DOI
10.1155/2009/852392; verified through search results)

CGA on historical corpora. The Xkipché paper "examines how architectural shape grammars can be used
to procedurally generate 3D reconstructions of an archaeological site", the Puuc-style Maya
buildings of Mexico — plinth, plain lower wall, ornate frieze with colonnettes and lattice, medial
and cornice mouldings; Rome Reborn generated the Roman city from a few grammars per building class,
with the argument that "both its efficiency and compactness make procedural modeling a tool to
produce multiple models which together sample the space of possibilities" where the archaeology is
uncertain.
*Bearing:* proof that CGA's split-and-insert vocabulary covers a non-European, pre-industrial style
with nothing added — a vertical split into bands and a repeated colonnette bay is D-039's `split`
and `repeat` over trim rows, so a Mesoamerican set is an attribute file — and the *uncertainty*
argument reused for fiction: a culture's grammar with ranges rather than values samples a plausible
space, which is what a seeded world wants. With Müller 2006's Pompeii (`city-generation.md` §3)
these are the Roman set's specification.

**Sven Havemann, Dieter W. Fellner. "Generative Parametric Design of Gothic Window Tracery."
*VAST 2004*, 193–201.** [paper] [foundational]
<https://diglib.eg.org/items/607768b5-11b0-4e72-9869-5d5df169f3f1> (verified through search
results)

Gothic tracery in the Generative Modeling Language: windows show "complex geometric shape
configurations achieved by combining only a few basic geometric patterns" (the pointed arch from
two circles, trefoil, quatrefoil, mullion subdivision), formalised so that "different combinations
of specific parametric features grouped together" create "style concepts".
*Bearing:* the *profile-and-construction* family of modules a split grammar cannot express —
compass constructions over an opening, mouldings swept along them, cusps as subtractions — and the
argument for a small construction language (arcs, offsets, sweeps, SDF booleans, `procedural.md`
§5) inside the module generator, shared by Gothic windows, Islamic arches, Roman arcades and a
habitat's viewport frames.

**Stefan Greuter, Jeremy Parker, Nigel Stewart, Geoff Leach. "Real-time procedural generation of
'pseudo infinite' cities." *GRAPHITE 2003*.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/604471.604490> (DOI 10.1145/604471.604490; verified through search
results)

Skyscrapers from a hash: "building generation parameters created by a pseudo random number
generator seeded with an integer derived from the building's position", the mass a stacked union
of random polygons with setbacks, textured with window grids, generated as the view requires.
*Bearing:* the modern high-rise is the cheapest style to generate — the stacked-setback mass with a
window grid *is* the style — and position-seeded parameters are D-016's discipline eleven years
early. The downtown set adds only its facade classes: curtain wall, spandrel-and-strip, punched
masonry, and brutalist precast, for which Banham's *The New Brutalism: Ethic or Aesthetic?*
(Architectural Press, 1966; "a scholarly history of the documentable facts on Brutalism"; verified
through search results) is the source — a precast module family, one board-formed concrete
tileable with streak weathering, deep reveals, a stacking-and-cantilever mass rule, and the natural
look of printed concrete and regolith bases.

**Patrik Schumacher. "Parametricism: A New Global Style for Architecture and Urban Design."
*Architectural Design* 79(4), 2009; and "Parametricism as Style — Parametricist Manifesto", Venice
Architecture Biennale, 2008.** [web] [still-current]
<https://patrikschumacher.com/parametricism-as-style-parametricist-manifesto/> (verified through
search results)

The manifesto of the biomorphic, computationally designed style associated with Zaha Hadid's
office, arguing that "the global convergence in recent avant-garde architecture justifies the
enunciation of a new style: Parametricism": continuous differentiation instead of repetition,
splines and fields instead of grids, every element correlated with its neighbours.
*Bearing:* the one contemporary style that breaks the module grid on purpose: a parametricist
landmark is a single generated mesh (a field-deformed lattice, a lofted surface) placed by
override. It is the right look for a futuristic civic set's landmarks, and its "nothing repeats"
rule is the sameness antidote stated as doctrine.

**"Greeble" — the film-model practice of detailing with kit parts (greeblies, nurnies,
kitbashing).** [web] [still-current]
<https://en.wikipedia.org/wiki/Greeble> (verified through search results)

Greebles are "small relief details used to give visual complexity to a model", a practice that
"originated as a technique in filmmaking" in the Star Wars model shop, where kit parts from tanks,
ships and aircraft were glued onto miniatures until unrecognisable; Frank Burton is quoted:
"Greeblie is a word George Lucas coined on Star Wars for something you can't otherwise define."
The "used future" those films established — scuffed, repaired, mismatched — is the same shop's
other legacy.
*Bearing:* the futuristic set's detail is a *distribution*, not a design: texture bombing in 3D
(`vegetation-materials.md` §4) with a small part library (vents, pipes, panels, conduits, lights)
over a hull, density and class from a mask, seeded per module so the same module never carries the
same greebles twice. The used future is §4's wear at full strength; the clean corporate future is
the same set with wear at zero.

**Richard D. Johnson, Charles Holbrow (eds.). *Space Settlements: A Design Study*. NASA SP-413,
1977 (technical director Gerard K. O'Neill); with O'Neill, *The High Frontier: Human Colonies in
Space*, William Morrow, 1976.** [book] [foundational]
<https://nss.org/settlement/nasa/75SummerStudy/Design.html> ·
<https://en.wikipedia.org/wiki/The_High_Frontier:_Human_Colonies_in_Space> (verified through search
results)

The engineering source for habitat architecture: the Stanford torus "capable of housing 10,000
permanent residents", "a ring with a diameter of about 1.8 km" rotating for about 1 g, from "a
10-week program in engineering systems design"; O'Neill's Island Three is a pair of cylinders,
"each cylinder is 5 miles (8.0 km) in diameter and 20 miles (32 km) long". The reports give
shielding mass, agricultural areas, window strips, population densities and town layouts.
*Bearing:* the far end of the era axis has a vernacular too: form follows spin gravity (a curved
floor), shielding (mass outside, windows as strips) and mass budgets (light interiors, terraces).
A habitat is a D-037 frame whose ground is a cylinder or torus, the 0.5 m grid applied on the
curved floor with the frame supplying the curvature and the hull one large DAG mesh; the reports'
population and area figures set the street density.

**A style set carries culture and era as data.** What each grammar above contributes is a *column*
of the style set, not a separate grammar:

| Column | From | Examples |
|---|---|---|
| plan rule | Stiny & Mitchell (grid), Duarte (patio dissection), Koning & Eizenberg (core and blocks), Knight (mat grid) | symmetric grid, patio dissection, hearth-core additive, tatami |
| module and grid | Li (rank → module), Knight (mat), Bethesda (footprint) | 0.5 m steps; bay of 4, 6 or 7 steps; floors 2.8–4.2 m |
| facade bands | Müller (Puuc, Pompeii), Haegler (Rome) | plinth / wall / frieze / cornice ratios, band materials |
| openings | Havemann (constructions), Kaplan (screens), Mahoney (ratio) | arch construction, screen family, opening % per side |
| roof | the climate table, Kelly & Wonka (`city-generation.md`) | pitch, eave depth, form, material |
| articulation | Flemming (towers, porches, per-floor material) | asymmetry probabilities, projection depth |
| ornament | Kaplan, Hamekasi, Havemann | tiling family and parameters; muqarnas plan; tracery construction |
| detail field | greebles | part library, density mask, wear strength |
| era | Greuter (setbacks), Banham (precast), Schumacher (landmark override), NASA (curved frame) | mass rule, facade class, landmark generator, frame kind |

Two records with the same split grammar and different columns are two cultures; the same culture
at two eras differs mostly in the last four rows. That is D-039's separation of the control grammar
from the split grammar, made concrete.

---

## 3. Decor, props and objects

A street or a room is read as inhabited by its objects, and the objects carry the culture more
legibly than the walls (an amphora, a shutter, a cable tray). Three layers of work exist:
generators for the objects, solvers for their arrangement, and the studio practice of dressing
from kits under a brief.

**Alexander Raistrick, Lingjie Mei, Karhan Kayan, David Yan, Yiming Zuo, Beining Han, Hongyu Wen,
Meenal Parakh, Stamatis Alexandropoulos, Lahav Lipson, Zeyu Ma, Jia Deng. "Infinigen Indoors:
Photorealistic Indoor Scenes using Procedural Generation." *CVPR 2024*; code in
princeton-vl/infinigen (BSD-3-Clause).** [paper] [code] [recent]
<https://openaccess.thecvf.com/content/CVPR2024/html/Raistrick_Infinigen_Indoors_Photorealistic_Indoor_Scenes_using_Procedural_Generation_CVPR_2024_paper.html>
(verified through search results) · <https://github.com/princeton-vl/infinigen> (README read)

The interior counterpart of the corpus `procedural.md` §4 names: "100% procedural, using no
external assets and using only mathematical rules to generate everything from scratch", with "a
diverse library of procedural indoor assets, including furniture, architecture elements,
appliances, and other day-to-day objects" — 79 generators, among them 17 for furniture, 10 for
appliances (112 parameters) and 14 for windows, doors and staircases (127 parameters) — and "a
constraint-based arrangement system, which consists of a domain-specific language for expressing
diverse constraints on scene composition, and a solver that generates scene compositions that
maximally satisfy the constraints". The repository is BSD-3-Clause and ships a transpiler
("`infinigen/nodes/node_transpiler/dev_script.py` provides tools to convert artist-friendly Blender
Nodes into python code").
*Bearing:* the most directly mineable source in this file: 79 generators with parameter
distributions are a *modern* civilisation's prop set written as code, and the constraint DSL
(against walls, on surfaces, facing, accessible, count ranges) is #88's room-dressing language. The
culture axis is what it lacks, so each generator gets a style block (material set, proportions,
ornament) rather than a new generator per culture.

**R. Kenny Jones, Theresa Barton, Xianghao Xu, Kai Wang, Ellen Jiang, Paul Guerrero, Niloy J.
Mitra, Daniel Ritchie. "ShapeAssembly: Learning to Generate Programs for 3D Shape Structure
Synthesis." *ACM Transactions on Graphics* 39(6) (SIGGRAPH Asia 2020), article 234.** [paper]
[code] [recent]
<https://rkjones4.github.io/shapeAssembly.html> (verified through search results) ·
<https://github.com/rkjones4/ShapeAssembly> (README read)

Objects as programs: "Executing a ShapeAssembly program produces a shape composed of a hierarchical
connected assembly of part proxies cuboids", with cuboid declarations, attachments between faces,
reflection and repetition; the repository provides "parsed ShapeAssembly datasets for chairs,
tables and storage categories" from PartNet. The learned part is optional.
*Bearing:* the representation for Forge's furniture and small-object generators: a program of
cuboids with attach, reflect and repeat, whose leaves are profile-swept or SDF-cooked parts — a few
hundred lines to interpret, deterministic, and what a culture parameterises (proportions, leg
count, back type, material). Furniture becomes a style set with the same columns as §2's.

**Paul Merrell, Eric Schkufza, Zeyang Li, Maneesh Agrawala, Vladlen Koltun. "Interactive Furniture
Layout Using Interior Design Guidelines." *ACM Transactions on Graphics* 30(4) (SIGGRAPH 2011); Lap-
Fai Yu, Sai-Kit Yeung, Chi-Keung Tang, Demetri Terzopoulos, Tony F. Chan, Stanley J. Osher, "Make
it Home: Automatic Optimization of Furniture Arrangement", same volume; Matthew Fisher, Daniel
Ritchie, Manolis Savva, Thomas Funkhouser, Pat Hanrahan, "Example-based Synthesis of 3D Object
Arrangements", *ACM Transactions on Graphics* 31(6) (SIGGRAPH Asia 2012).** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/2010324.1964982> (DOI 10.1145/2010324.1964982) ·
<https://dl.acm.org/doi/10.1145/2010324.1964981> (DOI 10.1145/2010324.1964981) ·
<https://graphics.stanford.edu/projects/scenesynth/> (DOI 10.1145/2366145.2366154; verified
through search results)

Layout from rules, from examples, and clutter. Merrell's system "incorporates the layout guidelines
as terms in a density function" — clearance, circulation, pairwise relations, alignment, balance —
"and generates layout suggestions by rapidly sampling the density function using a
hardware-accelerated Monte Carlo sampler"; Yu's "extracts hierarchical and spatial relationships
for various furniture objects" from furnished scenes into "priors associated with ergonomic
factors, such as visibility and accessibility", optimised "by simulated annealing using a
Metropolis-Hastings state search step"; Fisher's is "a probabilistic model for scenes based on
Bayesian networks and Gaussian mixtures that can be trained from a small number of input examples",
with objects clustered by their neighbourhoods so a desk learns what sits on desks.
*Bearing:* the guidelines are cultural (where the hearth is, whether one sits on the floor), so
the density terms are the culture's furniture rules and the sampler is shared; with a pinned seed
and a fixed iteration count it is deterministic (D-016). One solver with rule terms *and* example
terms, the example rooms being where the owner's hand authoring goes; and Fisher's occurrence model
per surface type is the small-object layer — cups, tools, papers — that no grammar wants to
enumerate, the same machinery over street surfaces being #87's dressing.

**Tobias Germer, Martin Schwarz. "Procedural Arrangement of Furniture for Real-Time Walkthroughs."
*Computer Graphics Forum* 28(8), 2009, 2068–2078; with Tutenel, Bidarra, Smelik, de Kraker,
"Rule-based layout solving and its application to procedural interior generation", *CASA 2009
Workshop on 3D Advanced Media in Gaming and Simulation*.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/j.1467-8659.2009.01351.x> (DOI
10.1111/j.1467-8659.2009.01351.x) · <https://graphics.tudelft.nl/Publications-new/2009/TBSD09a/>
(verified through search results)

The streaming form of the problem: Germer & Schwarz furnish "entire cities" by choosing "to only
furnish the rooms in the vicinity of the viewer while the user explores a building in real time",
with "an agent-based solution"; Tutenel et al. give "a novel rule-based layout solving approach,
especially suited for use in conjunction with procedural generation methods", with semantic
classes and relationships as the rules.
*Bearing:* what #88 must design around: rooms are dressed when they stream in, from the room record
and a seed, in a time budget (milliseconds per room), and the rules are semantic ("a table wants
chairs, a bed wants a wall") so they transfer across cultures with only the object set changed.
Forge dresses a room in a `Low` job when its cell opens and keeps nothing.

**Blender Foundation. "Instance on Points Node", Blender Manual; SideFX, "Copy to Points geometry
node" and "Executing tasks with PDG/TOPs", Houdini documentation.** [docs] [still-current]
<https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/instances/instance_on_points.html>
· <https://www.sidefx.com/docs/houdini/nodes/sop/copytopoints.html> ·
<https://www.sidefx.com/docs/houdini/tops/index.html> (verified through search results)

The node tools' shared primitive: Blender's node "adds a reference to a geometry to each of the
points present in the input geometry" ("instances are a fast way to add the same geometry to a scene
many times without duplicating the underlying data"); Houdini's copy "looks for specific attributes
on the destination points to customize each copy/instance", and PDG is the dependency-graph layer
"designed to distribute tasks and manage dependencies" across cores and farms.
*Bearing:* both place *attributes on points* and let the attributes choose and transform the copy,
which is Forge's placement pass already. The lesson is the attribute contract: a dressing rule emits
points with a class, a variant, a scale, a rotation and a wear value; `place_main` consumes them; a
culture's prop set is the table the class indexes.

**Naughty Dog. "Art Direction Bootcamp: Cinematic Environment Production for 'Uncharted 4'" and
"Technical Art Culture of 'Uncharted 4'", GDC 2017; Guerrilla Games, "Art Direction Bootcamp:
'Guerrilla Games' Approach to Asset Production", GDC 2016; KitBash3D's kit catalogue.** [talk]
[web] [still-current]
<https://www.gdcvault.com/play/1024310/Art-Direction-Bootcamp-Cinematic-Environment> ·
<https://gdcvault.com/play/1023251/Technical-Art-Culture-of-Uncharted> ·
<https://gdcvault.com/play/1023158/Art-Direction-Bootcamp-Guerrilla-Games> ·
<https://kitbash3d.com/products/mission-to-minerva> (verified through search results)

How the studios dress: Naughty Dog's environment talk is about "quieter, more intimate moments of
exploration through richly detailed environment spaces that tell a lot of the story", the
technical-art talk lists "automated runtime object population solutions" among its tools, and
Guerrilla's describes "highly detailed briefs required for their outsourcing approach". The
market's unit of a style is a themed kit: KitBash3D's free Mission to Minerva is "90 free 3D models
intended for kitbashing sci-fi buildings, machinery and vehicles", and its paid kits (Neo Tokyo,
Brutalist, Victorian, Wasteland, Medieval) are cut by era and mood.
*Bearing:* set dressing is storytelling, so the dressing rules carry a narrative parameter
(occupant, wealth, state: lived-in, abandoned, looted) that scales clutter, wear and object
classes; the *brief* is a real artefact, so a `Civilisation` record comes with a one-page brief and
a reference board; and 50–200 pieces per themed kit matches D-039's 150–400 modules per style set
and lists the styles players recognise.

**Signage, decals, vehicles and containers.** No paper treats these as culture, so the design
follows: signage is a glyph set per culture (SDF glyphs, `procedural.md` §5) on a sign module
family (hanging board, painted wall, neon tube, holographic panel) placed by D-039's storefront
rule; decals are the culture's marks (stencils, posters, ritual paint) in the regional decal atlas;
containers and vehicle parts are ShapeAssembly-style programs over the culture's material set, with
No Man's Sky's archetype-per-race scheme (§5) as the model. The Cyberpunk 2077 Night City talks in
`city-generation.md` are the shipped reference for signage density; none of it needs a new system.

---

## 4. Materials and textures by mathematics

`procedural.md` §5 carries the canon (Perlin 1985 and 2002, Worley 1996, Ebert et al. 2003, Cook &
DeRose 2005, Lagae 2009 and the 2010 survey, Heitz–Neyret 2018, Deliot–Heitz 2019, Burley 2019,
Mikkelsen 2022, Wronski 2025, Quilez's SDF articles, MATch 2020, Hu 2022) and
`vegetation-materials.md` §4–§5 the workflow (trim sheets, texture bombing, CC0 scans, BC formats
in KTX2, surface gradients). Forge ships hex-tiling (#66) and triplanar procedural textures shaded
by class (D-026). This section adds what a *culture's* materials need on top: warped and patterned
noise for man-made surfaces, reaction–diffusion for organic ones, spectral control, glints, the
node tools' vocabulary, by-example synthesis, the neural work as reference, and ageing.

**Inigo Quilez. "Domain warping." iquilezles.org, 2002 (updated).** [web] [foundational]
[still-current]
<https://iquilezles.org/articles/warp/> (verified through search results)

The one-line technique behind most "natural" procedural materials: "Warping simply means distorting
the domain with another function g(p) before evaluating f, replacing f(p) with f(g(p))", with fBm as
both the pattern and the distortion, so marble, wood grain, rust bloom, stucco and worn paint come
from the same two functions.
*Bearing:* the generator's first composite operator and the reason a material set needs few
primitives; also a *style parameter* (warp amplitude and frequency), since cultures' surfaces differ
as much in irregularity as in colour.

**Greg Turk. "Generating textures on arbitrary surfaces using reaction-diffusion." *Computer
Graphics* 25(4) (SIGGRAPH '91), 289–298; with Andrew Witkin, Michael Kass, "Reaction-diffusion
textures", same volume, 299–308.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/127719.122749> · <https://dl.acm.org/doi/10.1145/127719.122750>
(verified through search results)

Turk's "biologically motivated method of texture synthesis called reaction-diffusion", simulated
on the surface mesh so spots and stripes follow the geometry; Witkin & Kass add "anisotropic and
spatially non-uniform diffusion" and multiple diffusion directions.
*Bearing:* the organic patterns noise cannot make — hide, coral stone, lichen colonies, the
biomorphic panelling of a grown futuristic set, market fruit — as a Gray–Scott solver on a 512²
tile, milliseconds on the GPU at cook time; the generator to reach for when the owner asks for a
civilisation whose architecture is grown rather than built.

**Thibault Tricard, Semyon Efremov, Cédric Zanni, Fabrice Neyret, Jonàs Martínez, Sylvain Lefebvre.
"Procedural Phasor Noise." *ACM Transactions on Graphics* 38(4) (SIGGRAPH 2019).** [paper] [code]
[recent]
<https://dl.acm.org/doi/10.1145/3306346.3322990> (DOI 10.1145/3306346.3322990; verified through
search results) · <https://github.com/mfx-inria/phasornoise> (README read; AGPL-3.0)

Gabor noise's successor for *contrasted* patterns: the method "defines a stochastic smooth phase
field – a phasor noise – that is then fed into a periodic function like a sine wave", with "precise
control over the profile, orientation and distribution" of stripes, hatching, woven and brushed
patterns in a pixel shader. The reference code (Python and OpenCL) is AGPL, so a reference to read.
*Bearing:* the man-made textures noise does badly — woven mats and textiles, brushed metal, thatch
bundles, corrugated iron, plough furrows, the striations of printed concrete — from one phasor field
whose orientation map comes from the module's UV frame, which is what makes a trim's grain follow
the moulding.

**Xavier Chermain, Basile Sauvage, Jean-Michel Dischler, Carsten Dachsbacher. "Procedural
Physically based BRDF for Real-Time Rendering of Glints." *Computer Graphics Forum* 39(7) (Pacific
Graphics 2020), 243–253.** [paper] [code] [recent]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.14141> (DOI 10.1111/cgf.14141; code
<https://github.com/ASTex-ICube/real_time_glint>; verified through search results)

A BRDF that is procedural: it "procedurally computes NDFs with hundreds of sharp lobes" and
"converges to the standard microfacet BRDF for a large number of microfacets", so mica in granite,
frost, wet sand and flake paint sparkle under motion without a normal texture.
*Bearing:* one shading class in D-026's table (`glint`), parameterised per material row by
microfacet density and roughness — the quartz in a stone culture's ashlar, a desert's sand plaster,
a futuristic hull's paint — for a few dozen instructions per pixel where the class is set.

**Adobe. Substance 3D Designer documentation ("FX-Map", "Tile Sampler"); Rodolphe Suescun et al.,
Material Maker (GitHub, MIT); Blender Foundation, Shader Nodes ("Noise", "Voronoi", "Wave",
"Brick" textures), Blender Manual.** [docs] [code] [still-current]
<https://experienceleague.adobe.com/en/docs/substance-3d-designer/using/substance-graphs/nodes-reference-for-substance-graphs/atomic-nodes/fx-map>
· <https://github.com/RodZill4/material-maker> (README read) ·
<https://docs.blender.org/manual/en/latest/render/shader_nodes/textures/voronoi.html> (verified
through search results)

The node vocabulary. Designer's FX-Map "represents a special type of graph, known as a Markov Chain,
which represents a simple core process: repeatedly replicating and subdividing an image over and
over again" — recursive stamping, the engine of every brick, tile, scale and shingle generator —
and its Tile Sampler adds "seven different map slots which are available for driving Scale,
Position, Rotation, Size, Color and Masking". Material Maker is "a tool based on Godot Engine that
can be used to create textures procedurally and paint 3D models", MIT, whose "textures and brushes
are described as interconnected nodes" in readable GLSL; Blender's Voronoi node "evaluates a Worley
Noise at the input texture coordinates" and its Brick node is a parametric bond.
*Bearing:* Forge's generator needs two operators beside noise: a *recursive stamp* and a
*map-driven sampler*, because masonry — brick bonds, ashlar courses, flagstone, roof tiles,
shingles, panel grids — is what distinguishes cultures at a glance and masonry is a stamp with a
bond rule. Material Maker's MIT nodes are the open reference for a Slang port of a two-dozen-node
generator, and Blender's parameter names are the ones `MaterialGenerator` records should use.

**Alexei A. Efros, Thomas K. Leung. "Texture Synthesis by Non-parametric Sampling." *ICCV 1999*,
1033–1038; Vivek Kwatra, Arno Schödl, Irfan Essa, Greg Turk, Aaron Bobick, "Graphcut Textures",
*ACM Transactions on Graphics* 22(3) (SIGGRAPH 2003); Connelly Barnes, Eli Shechtman, Adam
Finkelstein, Dan B Goldman, "PatchMatch: A Randomized Correspondence Algorithm for Structural Image
Editing", *ACM Transactions on Graphics* 28(3) (SIGGRAPH 2009).** [paper] [foundational]
<https://www2.eecs.berkeley.edu/Research/Projects/CS/vision/papers/efros-iccv99.pdf> ·
<https://dl.acm.org/doi/10.1145/1201775.882264> ·
<https://gfx.cs.princeton.edu/pubs/Barnes_2009_PAR/patchmatch.pdf> (DOI 10.1145/1531326.1531330;
verified through search results)

By-example synthesis in three steps: Efros & Leung "grows a new image outward from an initial
seed, one pixel at a time" by matching neighbourhoods, "the degree of randomness is controlled by
a single perceptually intuitive parameter"; Graphcut Textures copies patches and stitches them
"along optimal seams", where "the size of the patch is not chosen a-priori, but instead a graph cut
technique is used to determine the optimal patch region"; PatchMatch is "a new randomized algorithm
for quickly finding approximate nearest-neighbor matches between image patches", "20-100x" faster,
which made all of it interactive.
*Bearing:* the cook-time path for materials Forge cannot write as functions — a scanned plaster,
the owner's reference — turned into a tileable once and cached by the exemplar's hash; graph-cut
synthesis for the tile, PatchMatch inpainting of the wrap-around border to make a CC0 photograph
seamless, and hex-tiling to hide what remains at run time.

**Michael F. Cohen, Jonathan Shade, Stefan Hiller, Oliver Deussen. "Wang Tiles for Image and
Texture Generation." *ACM Transactions on Graphics* 22(3) (SIGGRAPH 2003), 287–294.** [paper]
[foundational]
<https://dl.acm.org/doi/10.1145/1201775.882265> (DOI 10.1145/1201775.882265; verified through
search results)

"A simple stochastic system for non-periodically tiling the plane with a small set of Wang Tiles",
whose tiles "may be filled with texture, patterns, or geometry", so "large expanses of non-periodic
texture (or patterns or geometry) can be created as needed very efficiently at runtime".
*Bearing:* the alternative to hex-tiling for *structured* materials where blending would smear the
structure (brick, cobble, tile roofs, panel grids): eight to sixteen tiles with matching edges,
synthesised once with the edge constraint and chosen per cell by a hash. The "or geometry" clause
is the point: a paving module family with matching edges is a Wang set, which is how a plaza gets an
irregular pattern from a dozen instanced pieces.

**Yu Guo, Cameron Smith, Miloš Hašan, Kalyan Sunkavalli, Shuang Zhao. "MaterialGAN: Reflectance
Capture using a Generative SVBRDF Model." *ACM Transactions on Graphics* 39(6) (SIGGRAPH Asia
2020); Gilles Rainer, Wenzel Jakob, Abhijeet Ghosh, Tim Weyrich, "Neural BTF Compression and
Interpolation", *Computer Graphics Forum* 38(2), 2019, 235–244; Paul Guerrero, Miloš Hašan, Kalyan
Sunkavalli, Radomír Měch, Tamy Boubekeur, Niloy J. Mitra, "MatFormer: A Generative Model for
Procedural Materials", *ACM Transactions on Graphics* 41(4) (SIGGRAPH 2022); Yiwei Hu, Julie Dorsey,
Holly Rushmeier, "A Novel Framework for Inverse Procedural Texture Modeling", *ACM Transactions on
Graphics* 38(6) (SIGGRAPH Asia 2019).** [paper] [recent]
<https://shuangz.com/projects/materialgan-sa20/> ·
<https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.13633> (DOI 10.1111/cgf.13633) ·
<https://dl.acm.org/doi/abs/10.1145/3528223.3530173> (DOI 10.1145/3528223.3530173) ·
<https://graphics.cs.yale.edu/publications/novel-framework-inverse-procedural-texture-modeling>
(verified through search results)

The neural line in four steps: MaterialGAN is "a deep generative convolutional network based on
StyleGAN2, trained to synthesize realistic SVBRDF parameter maps", a prior for capture from phone
photographs; neural BTFs capture "subtle surface variations and anisotropy that are lost by
principal component analysis-based compression at the same compression ratio"; MatFormer generates
procedural *graphs*, motivated by the fact that "publicly accessible libraries contain only a few
thousand such graphs"; Hu et al. give "an example-based framework to automatically select procedural
models and estimate parameters" from a photograph.
*Bearing:* reference, not dependency, for the reasons Forge has stated (P3/P8, D-016, editable
data): a network's output is not a pure function of a seed across GPUs and drivers, none of it runs
on the server, and a material must stay a record. What the line proves is that a few thousand graphs
span the world's materials and that a photograph can be fitted to one automatically, so Forge's
vocabulary can be small and the fitting (MATch, Hu 2022) is an offline tool whose output is
parameters.

**Julie Dorsey, Pat Hanrahan. "Modeling and Rendering of Metallic Patinas." *SIGGRAPH '96*; Dorsey,
Hans Køhling Pedersen, Hanrahan, "Flow and Changes in Appearance", *SIGGRAPH '96*, 411–420; Dorsey,
Alan Edelman, Henrik Wann Jensen, Justin Legakis, Pedersen, "Modeling and Rendering of Weathered
Stone", *SIGGRAPH '99*, 225–234.** [paper] [foundational]
<https://graphics.cs.yale.edu/publications/modeling-and-rendering-metallic-patinas> ·
<https://history.siggraph.org/learning/flow-and-changes-in-appearance-by-dorsey-pedersen-and-hanrahan/>
· <http://graphics.ucsd.edu/~henrik/papers/sig99/> (verified through search results)

The founding weathering papers. A patina is "a film or incrustation on a surface produced by the
removal of material, the addition of material, or chemical alteration of a surface", modelled as
layers grown by operators (coat, erode, fill, polish) over time; "Flow" simulates water running
over a model, depositing and dissolving, so streaks form under sills and stains pool where flow
slows; weathered stone is a volume — a "slab" data structure, "a surface-aligned volume confined to
a narrow region around the boundary of the stone", with "the flow of moisture and the transport,
dissolution, and recrystallization of minerals within the porous stone volume" — so edges round,
crusts form and carvings soften.
*Bearing:* ageing is a *process on the geometry*, not a decal. Forge runs the flow step once per
module at cook (the module knows its ledges), writes a `wear` mask into the module's texture set
and scales it at run time by the civilisation's age and the site's rainfall from the atlas; copper,
bronze and iron patinas are layer stacks in the palette; and the slab is the ruin generator — a
civilisation's stone modules at age *t* are the fresh module minus a heightfield of loss applied as
displacement before the cook, so a ruined temple set is one parameter over the fresh set rather
than a second kit.

**Yanyun Chen, Lin Xia, Tien-Tsin Wong, Xin Tong, Hujun Bao, Baining Guo, Heung-Yeung Shum.
"Visual Simulation of Weathering by γ-ton Tracing." *ACM Transactions on Graphics* 24(3) (SIGGRAPH
2005), 1127–1133; Jiaping Wang, Xin Tong, Stephen Lin, Minghao Pan, Chao Wang, Hujun Bao, Baining
Guo, Heung-Yeung Shum, "Appearance Manifolds for Modeling Time-Variant Appearance of Materials",
*ACM Transactions on Graphics* 25(3) (SIGGRAPH 2006).** [paper] [foundational] [still-current]
<https://dl.acm.org/doi/abs/10.1145/1073204.1073321> (DOI 10.1145/1073204.1073321) ·
<https://www.microsoft.com/en-us/research/publication/appearance-manifolds-for-modeling-time-variant-appearance-of-materials/>
(verified through search results)

Exposure, then appearance. In γ-ton tracing, "aging-inducing particles called γ-tons" are emitted
from sources (the sky for rain and dust, the ground for splashes and moss), traced like photons,
and their deposits drive "dirt, rust, cracks and scratches" through per-material rules, so exposure
and shelter fall out of the geometry; appearance manifolds age from one photograph, since
"concurrent variations in appearance over a surface represent different degrees of weathering", so
ordering the patches of one weathered sample by degree yields a one-dimensional manifold of
appearance over time, applied to a fresh surface by a degree map.
*Bearing:* the scene-scale ageing pass, runnable on D-029's TLAS — a few thousand rays from above
per building at cell load deposit exposure in a per-instance wear channel, so a courtyard's
sheltered side stays clean and the weather side streaks, with contact wear (handles, thresholds,
worn steps) the same with sources at the walkable slabs the modules tag — and the cheapest
appearance model to ship: per material row a small *appearance strip* (albedo, roughness and normal
statistics against degree), indexed in the resolve by the wear channel, authored by the generator
or fitted from a scan; three scalars per pixel and one strip lookup age every culture's materials
without a second texture set.

**Stéphane Mérillou, Djamchid Ghazanfarpour. "A survey of aging and weathering phenomena in
computer graphics." *Computers & Graphics* 32(2), 2008, 159–174; with Julie Dorsey, Holly
Rushmeier, François Sillion, *Digital Modeling of Material Appearance*, Morgan Kaufmann, 2008
(chapter 8, "Aging and Weathering").** [paper] [book] [still-current]
<https://www.sciencedirect.com/science/article/abs/pii/S0097849308000058> (DOI
10.1016/j.cag.2008.01.003) ·
<https://www.sciencedirect.com/book/monograph/9780122211812/digital-modeling-of-material-appearance>
(verified through search results)

The map of the ageing literature by phenomenon (corrosion, patina, dust, cracks, peeling,
biological growth, erosion, contact wear) and method, noting that "aging processes result from
materials' composition, objects' wear, weathering conditions" and other parameters; the book
chapter is the textbook form.
*Bearing:* the checklist for the `WearProfile` record — which phenomena a material row admits
(iron rusts, whitewash peels and greens, timber greys and cracks, adobe erodes, composites chalk)
and which the civilisation's climate and age select. Lagarde's wet response (`planet-environment.md`
§4, D-034) is the short-term end of the same axis and shares the porosity field.

**Material sets per culture: local resources to palettes.** The regional material set of
`vegetation-materials.md` (2–4 trim sheets, 6–10 tileables, one decal atlas, vertex-paint masks) is
the unit; a `Civilisation` derives its set from resources and era and names the few overrides that
carry identity (the Provençal ochre, the Greek blue, the Japanese charred cedar):

| Resources | Era | Tileables (generator families) | Trims | Wear profile |
|---|---|---|---|---|
| clay, straw, little timber | primitive–medieval | adobe (warped noise + cracks), whitewash (thin coat), thatch (phasor bundles), packed earth | timber lintels, carved frames | erosion at the base, whitewash peeling, thatch greying |
| limestone, timber, clay | medieval–early modern | rubble and ashlar (stamp with bond), lime plaster (ochre range), canal tile (stamp), oak planks (warped grain) | génoise cornice, quoins, sills, shutters | streaks under sills, lichen bombs on the north side, worn steps |
| slate, granite, peat | cold vernacular | drystone (Voronoi cells), slate (stamp), sod (grass detail), log (revolved profile) | eave boards, corner posts, snow guards | moss in joints, greyed timber, frost spall |
| steel, glass, fired brick | industrial–modern | brick bonds (stamp), board-formed concrete (phasor stripes + noise), painted steel, corrugated iron (phasor), asphalt | spandrels, mullions, cornices, sign rails | rust runs from fixings, soot gradients, chalking paint, graffiti decals |
| regolith, printed metal, composites | futuristic | sintered regolith (Worley + dust), printed-concrete striations, panel grids (Wang set), fabric (phasor weave) | panel seams, conduit runs, viewport frames, handrails | micrometeorite pitting, scuffs at handholds, dust drifts, decal fading |

Every row is the same vocabulary (noise, warp, Worley, stamp, phasor, reaction–diffusion,
by-example) with a palette and a wear profile; a culture with unusual resources (coral, bone, ice,
glass sand) is a new row, not a new tool.

---

## 5. What the shipped games do with civilisations

The line that matters is the same as in the other files: what is hand-set and what is generated.
Every game below hand-sets a small number of *sets* per culture and generates, if at all, within
them; the data-driven ones keep the sets in files a mod can replace, which is D-035's thesis.

**Firaxis Games. *Sid Meier's Civilization VI* (2016): district and city-centre architecture by
culture group and era — Civilization Wiki.** [web] [weak]
<https://civilization.fandom.com/wiki/Palace_(Civ6)> (verified through search results; a fan wiki,
no developer talk on the architecture sets was found)

"The architecture of these districts varies by civilization's cultural group", buildings change
across the eras, and the shared Palace models are regional — the one "shared by the Indians, the
Arabs, the Persians and the Scythians is a Mughal-style building inspired by the Humayun's Tomb",
the Khmer and Indonesian one "inspired by Prasat Bayon and the gates of Angkor Thom".
*Bearing:* dozens of civilisations from a *handful* of culture groups crossed with eras, plus a few
unique landmarks per civilisation — the cheapest honest scheme for many cultures, and the one the
recommendation adopts for the many-culture case.

**Ensemble Studios, Forgotten Empires. *Age of Empires II* (1999–, Definitive Edition 2019–):
architecture sets; Relic Entertainment, World's Edge, *Age of Empires IV* (2021): per-civilisation
architecture over four ages — Age of Empires Series Wiki and the official civilisation pages.**
[web] [still-current]
<https://ageofempires.fandom.com/wiki/Architecture_set_(Age_of_Empires_II)> ·
<https://www.ageofempires.com/games/age-of-empires-iv/civilizations/english/> (verified through
search results)

In II, "starting in the Feudal Age, building appearances change into one of eleven possible
architecture sets, depending on the civilization and its real-life geographical area" (the count
moves with expansions) — Western European "based on a mixture of medieval British Tower houses,
Norman architecture, and Anglo-Saxon architecture", Central European, Mediterranean, Eastern
European, Middle Eastern, Central Asian, East Asian, Southeast Asian, South Asian, African, Native
and South American — the same building roster redrawn per set and each civilisation's wonder
unique. In IV each civilisation has its own architecture in each of four ages — "the English
civilization progresses through the Anglo-Saxon, Anglo-Norman, English Gothic, and Tudor eras" —
while "most civilizations share the same roster of standard buildings" and advance through
"bespoke buildings called landmarks".
*Bearing:* the clearest statement of "same roster, different set": one list of building *classes*
shared by all cultures, one style set per culture and era that gives every class its look, one
landmark generator per culture. Eleven-plus sets over twenty-five years, and four ages per
civilisation in IV, also say what a set costs a studio — months of a small art team each — which is
the budget the generators replace and the reason the era rows of §2's table must be cheap.

**Bay 12 Games. *Dwarf Fortress*: the raw files (`material_template`, `entity_default`, ethics and
values) — Dwarf Fortress Wiki.** [web] [docs] [still-current]
<https://dwarffortresswiki.org/index.php/DF2014:Raw_file> ·
<https://dwarffortresswiki.org/index.php/DF2014:Entity_token> (verified through search results)

Cultures and materials as text: "ENTITY defines civilization types, with assigned race, language,
culture, ethics, and social structure", each beginning with "[ENTITY:entity_ID]" and specified by
further tokens (permitted buildings, jobs, clothing, weapons, values and ethics); "MATERIAL_TEMPLATE
defines information common to groups of materials", inherited by stones, metals, woods and tissues.
*Bearing:* the closest shipped analogue to D-035 applied to civilisations: a culture names its
permitted things and its values, a material is a record everything inherits from, and mods replace
either. Forge's `Civilisation` needs Dwarf Fortress's *permission lists* (which building classes,
prop classes, materials and crafts a culture uses), because a culture is defined as much by what it
never builds.

**Ludeon Studios. *RimWorld* (2018) and *Ideology* (2021): the Stuff system and the ideoligion
styles — RimWorld Wiki.** [web] [docs] [still-current]
<https://rimworldwiki.com/wiki/Stuff> · <https://rimworldwiki.com/wiki/Ideology_(DLC)> (verified
through search results)

"Stuff is a concept in RimWorld's code by which a material can be chosen before creation of an item
or building", the material's stats setting the product's, with `StuffProperties` carrying
"smeltable, appearance (StuffAppearance), soundImpactStuff, statOffsets" and categories. Ideology
adds *styles* — Spikecore, Morbid, Techist, Rustic, Totemic — that reskin the same buildings and
items with a culture's motifs ("Morbid features tiles and carpets with horror skulls, Spikecore has
tiles and plates, Totemic has tile and boards, and Techist has hex carpets and tiles").
*Bearing:* two mechanisms to copy literally. *Stuff* is D-007's material row used as a construction
parameter — a granite wall and a steel wall are one class and two rows — and *styles* are a
`motifs` column that swaps ornament, decal and palette per class without a new kit. RimWorld sold
five such styles as a DLC's main feature, which says how much players read from motifs alone.

**Guerrilla Games. *Horizon Forbidden West* (2022): the tribes' settlements — "Horizon Forbidden
West: An authentic world", PlayStation Blog, 22 November 2021.** [web] [recent]
<https://blog.playstation.com/2021/11/22/horizon-forbidden-west-an-authentic-world/> (verified
through search results)

For the Utaru, "settlements are made of wood and rope, featuring minimal furnishings other than
what is needed for daily life", and across tribes "culture relies on craftsmanship and artisanal
traditions that give recognizable cultural identities to groups through things like clothing styles
and architecture details"; the Tenakth's "beliefs are influenced by the ancient ruins of the
Forbidden West". Kenshi (Lo-Fi Games, 2018) is the small-team case of the same practice: four
faction kits — Holy Nation, United Cities, Shek, Hive — placed by hand and told apart at a glance
(Kenshi Wiki; verified through search results, no developer source).
*Bearing:* the AAA version of Rapoport: each tribe is a *brief* (resources, beliefs, craft) from
which architecture, costume and props follow, and cultures differ in what they build *with* — rope
and wood, salvaged machine parts, the ruins themselves. Salvage deserves a column: a culture's
material set can include another era's leftovers, which is how a post-collapse or colonial style is
written as data.

**BioWare, *The Art of the Mass Effect Universe* (Dark Horse, 2012) as excerpted by the Mass Effect
Wiki; Cloud Imperium Games, *Star Citizen*: ship manufacturers — Star Citizen Wiki.** [web] [weak]
<https://masseffect.fandom.com/wiki/The_Art_of_the_Mass_Effect_Trilogy/Mass_Effect> ·
<https://starcitizen.tools/Ship_manufacturers> (verified through search results; fan wikis quoting
the art book and the lore)

Species and manufacturers as design languages: for Sur'Kesh the art team wanted "architecture
inspired by a shopping center in Istanbul", interiors that "blur the line between landscape and
structure"; in Star Citizen "each manufacturer has a style and goal that shapes the ships they
design, from Drake Interplanetary's rough-and-tumble design to MISC's rugged reliability", about ten
majors each owning a silhouette, a panel language, a palette and an interior kit.
*Bearing:* the futuristic equivalent of a culture is a manufacturer or a species, written the same
way as a `Civilisation` — silhouette rules, seam density, palette, interior kit, wear — and ten
languages over a few hundred ships is the ratio to plan for in the space battle (#80).

**Grant Duncan (Hello Games). "Art Direction Bootcamp: How I Learned to Love Procedural Art." GDC
2015; with No Man's Sky's starship archetypes — No Man's Sky Wiki.** [talk] [web] [still-current]
<https://www.gdcvault.com/play/1021805/Art-Direction-Bootcamp-How-I> ·
<https://nomanssky.fandom.com/wiki/Starship> (verified through search results)

The talk covers "how a tiny team of artists at Hello Games used procedural technology to create the
infinite worlds of No Man's Sky, and how to maintain quality and artistic control when faced with
the chaos of infinity", making "different objects from the same base model" under rules. The ships
are "six different ship archetypes: Fighter, Hauler, Explorer, Shuttle, Exotic, and Living Ship",
distributed by race — "each Gek system has 7 Haulers, 3 Explorers and 3 Fighters", the Korvax seven
Explorers, the Vy'keen seven Fighters — with wings, cockpits and thrusters combined per system.
*Bearing:* archetype-with-parts is what Forge's vehicle and prop generators should be, and the
*distribution* per race is the culture's signature as much as the parts: a `Civilisation` carries
class *frequencies* (shrines per hundred houses, carts per market) beside its kits.

**Bethesda Game Studios. *Starfield* (2023): hand-built cities over Bethesda's kit doctrine,
procedural planets with hand-made points of interest — press coverage.** [web] [weak]
<https://www.pcgamesn.com/starfield/procedural-generation> (verified through search results; no
developer talk on the cities' kits was found)

As reported, "cities aren't procedurally generated and are hand-crafted with characters and
missions", and "points of interest on these planets emerge from a curated selection of hand-crafted
spaces"; the kits are the Skyrim and Fallout 4 doctrine (`city-generation.md` §3) with a
science-fiction set per faction.
*Bearing:* a thousand planets and three hand-built cities is the studio answer when the kits exist
but the assembly is manual. Forge's argument is that the assembly is what the grammar automates, so
the culture can live in every settlement; Starfield's factions (frontier, corporate, military) are
a fair list for the first futuristic sets.

**Ubisoft. Assassin's Creed's historical research: Maxime Durand, franchise historian —
interviews (Game Developer, TheSixthAxis, History Respawned, 2017–2018).** [web] [still-current]
<https://www.gamedeveloper.com/game-platforms/interview-with-maxime-durand-on-assassin-s-creed-origins-and-discovery-tour-mode>
(verified through search results)

The historian's work "follows cycles that include basic historic research during pre-conception
covering historical timelines, events, main characters and locations, followed by more precise
questions during production about what people wore, their occupations, technology, and language",
and the studio "maintains a large internal database of videos and books for research"; the
buildings are kits per district and era (`city-generation.md`'s Unity entry).
*Bearing:* the authoring workflow for a *real* culture — brief first, kit from the brief — and its
two phases map onto Forge's two passes: §2's coarse columns first, §3's props and decals when the
district is dressed. The brief is the `Civilisation` record's prose companion.

**Mojang Studios. *Minecraft*: villages by biome since Village & Pillage (1.14, 2019), structure
templates and jigsaw blocks — Minecraft Wiki; misode/mcmeta, the vanilla data mirror (GitHub).**
[web] [docs] [code] [still-current]
<https://minecraft.wiki/w/Village/Structure> (verified through search results) ·
<https://github.com/misode/mcmeta> (README read)

"The type of the village, and therefore the style of all structures within it, is determined by the
biome at the village center or meeting point": plains villages "made of oak logs, oak planks,
cobblestone", taiga and snowy "use spruce wood", savanna "acacia wood", desert "sandstone instead",
the snowy taiga reusing the taiga set under snow. Each type is a pool of structure templates
(`/data/minecraft/structures/village`) assembled by jigsaw blocks, all in the data pack; mcmeta is a
"processed, version controlled history of Minecraft's generated data and assets" where the pools
can be read per version.
*Bearing:* five biome sets, each a pool of small templates with typed connectors and weights, is the
smallest shipped style set and entirely data. Copy the *weights* per template (the frequencies the
No Man's Sky entry asked for) and the fallback rule (a biome without a set borrows a neighbour's and
changes the dressing), which is how Forge handles a culture whose set is not built yet.

**What they hand-author and what they generate.**

| Game | Culture unit | Hand-authored | Generated | Era axis |
|---|---|---|---|---|
| Civilization VI | culture group | every model per group and era; unique landmarks per civ | city layout on the hex grid | per group, several eras |
| Age of Empires II / IV | architecture set / per civ | every building per set; wonders and landmarks | nothing | four ages |
| Dwarf Fortress | ENTITY record | the records (text), tile art | sites, layouts, history, artefacts | none |
| RimWorld | style (motif set) | defs and art per style | maps, events | none |
| Kenshi | faction kit | kits and town placement | nothing | none |
| Horizon Forbidden West | tribe brief | everything, from the brief | vegetation placement | none, plus ruins |
| No Man's Sky | race + archetype | base and archetype parts | combinations, distributions | none |
| Starfield | faction kit | the cities, the POI interiors | planet surfaces, POI placement | none |
| Assassin's Creed | district/era kits | everything, from the historian's brief | crowds, placement | one era per game |
| Minecraft | biome template pool | the templates | village assembly by jigsaw | none |

No shipped game generates the *set*; all generate, at most, the assembly. That is the gap Forge's
module generators fill, and the reason the risk of sameness (§6) is Forge's and not the industry's.

---

## 6. How it all fits Forge

**The `Civilisation` record is a package layer over style sets.** D-039 gives a style set per
district — module generators and parameters, control-grammar attributes, a regional material set,
a contrast rule. A civilisation does not replace it; it *selects and patches* (D-035's `Patch`)
several at once — one per building class (dwelling, workshop, shrine, hall, wall, tower) — plus a
prop set, a decal and glyph set, a furniture set, a wear profile and class frequencies, and carries
the columns §2 derived. Its climate response is computed from the atlas at the settlement's
founding cell and stored, so an author overrides fields, never the derivation. Two civilisations of
one culture at different eras share most columns and differ in the era rows; a city founded by one
and grown under another has a `founded` age per district (already in `city-generation.md`'s
`District`) that selects the older record in the core and the newer in the rings, with the wear
profile scaled by age.

**The 0.5 m module grid, from a hut to a habitat, and where it breaks.** It holds for more than it
might seem: a wattle hut is a ring of 1 m panels and a conical roof module; a longhouse, a Roman
insula, a machiya (with the 1.8 m bay as 3.6 steps, the test case), a Provençal house, a
curtain-wall tower, a brutalist block, a station corridor all sit on it; a habitat's curved floor is
the grid inside a cylindrical D-037 frame whose curvature the frame supplies. It breaks in three
places, each with its own path: *tents and yurts* (revolved or lofted profiles: single generated
meshes placed as props, one "shelter" class of the prop set); *organic and hand-shaped forms*
(adobe curves, mud domes, grown biomorphic sets: Townscaper's deformed quads at village scale as
`city-generation.md` §6 allows, and SDF-cooked single meshes for domes and landmarks); and
*megastructures* (a torus hull, a dam, a launch tower: one large DAG mesh like the terrain, with the
grid applied only on its inhabited surfaces). Parametricist landmarks are overrides, as §2 said.

**The texture pipeline.** Materials are records naming a generator graph (the two dozen operators
of §4: noise, warp, Worley, stamp, phasor, reaction–diffusion, by-example, flow and patina) and its
parameters, plus a palette. At cook time `forge-procgen::material` evaluates the graph to tileables
and trim strips at the style's resolution, runs the flow and γ-ton wear pass per module, compresses
to BC7/BC5/BC4 in KTX2 (`vegetation-materials.md` §5) and stores them under the BLAKE3 hash of
graph + parameters + palette + resolution (D-018's cache), so a palette change re-cooks one style
set's textures and nothing else. The cook runs on the GPU where present (a 2048² graph of a few
hundred operations per texel is well under a millisecond on the 5070 Ti; BC7 encoding is the slow
step, seconds per texture on the CPU) and its output is cosmetic, so D-016's cross-machine
determinism is required of the *parameters* and the cache key, not of every texel; the server never
cooks textures. Run time is unchanged: hex-tiled tileables (#66), trims by UV snap from the
grammar's tags, decals bombed, wear read from the instance's channel and the appearance strip, all
in the visibility-buffer resolve.

**Costs, estimated and to be measured.** *Textures:* at 2048² a material of three maps (albedo
BC7, normal BC5, ORM/height BC7 or BC4) is about 16 MiB with mips; a style set of 4 trims, 10
tileables and a decal atlas is 15 materials, ≈ 240 MiB at 2K or ≈ 60 MiB at 1K, and the sensible mix
(trims at 2K because they carry shape, tileables at 1K because hex-tiling and a detail normal carry
them) lands at 100–150 MiB per style set; a civilisation of four building-class sets sharing one
regional material set stays under 200 MiB; ten civilisations resident would be 1–2 GiB without the
virtual texture, which is why the material pages go through the same page pool as the clusters
(D-025's 128 KiB pages). *Modules:* a primitive set is 40–80 modules, a medieval or modern one
150–300, a futuristic one 150–300 plus 30–60 greeble parts; at 5–50 k triangles a module and today's
cook rate (8 M triangles in 12 s, `city-blocks.md`), a 300-module set cooks in 10–20 s and occupies
100–150 MiB of cluster pages (the city's props are 983 MiB for 25.8 M triangles, about 40 bytes a
triangle). *Generation:* the response derivation is microseconds; the material cook seconds to a
minute per set; the module cook tens of seconds per set; room dressing milliseconds per room in a
`Low` job; all cached. *Per era:* the number of unique modules rises with the era's complexity, but
the number of *generator families* does not — perhaps twenty (wall, opening, roof, corner, cornice,
column, stair, balcony, screen, panel, conduit, and so on) cover every era with different
parameters.

**What the visibility-buffer renderer and the cluster DAG need.** From D-026's table: a material
row per module section (D-027), the shading classes `standard`, `glass`, `glint` and `layered`
(weather and wear over a base), and a per-instance *palette index* and *wear scalar* in the 80-byte
instance row (a `u16` and a `u8`, to be checked against its padding), so a module's texture set is
shared across civilisations that differ only in palette and age. From the DAG: nothing new for
modules (they are props), a merged proxy per building (D-039), and the fact that geometric ornament
(muqarnas, tracery, greebles, friezes) can be geometry because clusters LOD it and the software
rasteriser draws it at a pixel — the argument for cooking ornament as mesh rather than faking it in
normal maps. From the placement pass: §3's attribute contract on the points the dressing rules emit.

**The risk of sameness, and the antidotes.** The failure the owner will see first is "the same
generator in a different palette": every culture's houses the same proportions, every wall the same
noise, every street the same rhythm. The antidotes, in order of cost: *asymmetry as rules*
(Flemming's picturesque attributes and Schumacher's "nothing repeats" as a repetition penalty in
the grammar's choice of variants); *landmarks* (Lynch's nodes from `city-generation.md`; each
culture's sacred and civic building generated by a different plan rule than its dwellings and placed
at the focal point, so the skyline is the culture's before any facade is read); *hand-authored
overrides* (D-035 records for a few hero buildings and props per culture, as Civilization gives each
civilisation a unique wonder, and the example rooms that train §3's priors); *wear* (the same
module aged differently by exposure, contact and the civilisation's age is the cheapest variety
there is, and what makes a world read as inhabited); and *the metrics* below, so sameness is
measured before it is noticed.

---

## Recommendation for Forge

**Start small, prove both axes.** D-039 names two style sets (a Mediterranean village for #91 and
a downtown for city-blocks). The plan builds the civilisation layer over them, adds one primitive
and one futuristic set to prove the era axis, and only then widens the culture axis.

1. **`ClimateResponse` from the atlas.** The Mahoney tables and Givoni's zones as a function of a
   D-034 atlas cell (twelve months of temperature, humidity, diurnal range, rain) to a struct:
   class (hot-arid, hot-humid, temperate, cold), opening ratio, wall mass class, roof pitch range,
   courtyard and stilt flags, orientation, eave depth, compact-or-spread. Unit tests against the
   tables' worked examples and against the island's cells. A day.
2. **The `Civilisation` record and its merge.** `forge-data` records for `Civilisation`,
   `StyleSetRef` per building class, `PropSet`, `GlyphSet`, `WearProfile`, `Palette`, class
   frequencies and permission lists; a `Civilisation` layers over D-039's style sets by D-035
   `Patch`; two records for the two existing sets (a Provençal village culture, a contemporary
   downtown), each with a one-page brief. The merged records' digest enters D-016's set.
3. **The material generator.** `forge-procgen::material`: the operator set (noise, fBm, warp,
   Worley, recursive stamp, phasor, reaction–diffusion, palette map, height-to-normal), graphs as
   RON, evaluated to tileables and trim strips, BC-compressed into KTX2 and cached by content hash;
   the two regional material sets written as graphs (rubble, lime plaster, canal tile, oak; curtain
   glass, spandrel, concrete, asphalt), hex-tiled at run time; a `materials --set provencal` tool
   that renders a contact sheet, judged by the owner against the CC0 scans it should replace.
4. **Wear.** The flow pass per module at cook (streaks under ledges, dirt at the base), γ-ton
   exposure per instance from the TLAS at cell load, contact wear from walkable slabs, an appearance
   strip per material row, the `wear` scalar in the instance row read by the resolve; `--age
   0|50|300` on the village to show one set fresh, weathered and ruined.
5. **The primitive set.** Forty to eighty modules (post, wattle panel, adobe wall, thatch and hide
   roofs, palisade, hearth, ladder, platform) and the hearth-core additive plan rule; a
   `Civilisation` for a hot-humid stilt village and one for a hot-arid adobe village from the same
   set with different responses and palettes, placed on the island by the village pipeline — the
   cheapest proof that the grid holds for the crudest architecture and that two climates read
   differently from one kit.
6. **The futuristic set.** Panels, seams, conduits, viewport frames, handrails, doors and airlocks
   as modules; 30–60 greeble parts and the bombing rule; a "used" and a "clean" `WearProfile`; a
   corridor and hangar interior on the same grid (#88's first sci-fi rooms); an asteroid base carved
   into one of the belt's big rocks (#80's reference look), with the curved-floor habitat frame as a
   spike.
7. **Props and dressing.** The ShapeAssembly-style program interpreter for furniture and
   containers, a first prop set per civilisation (twenty to forty programs), the room dressing rules
   (Merrell's terms) with a pinned-seed sampler in a `Low` job at cell open, Fisher-style clutter from
   a dozen authored rooms, signage from an SDF glyph set; street dressing (#87) with the same
   machinery over the roads' profiles.
8. **The culture axis.** Only now: a third and fourth culture on the medieval set (a cold timber
   culture; an Islamic-influenced courtyard culture with Kaplan's screens and a muqarnas module) to
   measure how much of a culture is data (target: no new module generators, a new material graph
   set, a new plan rule at most) and to fix the authoring time per culture.

**The data model, as Rust records** (names to be argued in the code):

```
Civilisation { id: "pkg:civ/provencal", culture: CultureId, era: Era,
               response: ClimateResponse,           // derived from the atlas at founding; overridable
               resources: [ResourceId],             // timber, limestone, clay, steel, regolith …
               materials: RegionalMaterialSetId,     // trims, tileables, decal atlas (graphs + palette)
               palette: Palette { base, accent, roof, trim, sign },
               styles: { BuildingClass -> StyleSetId },   // D-039 sets, patched
               grammar: AttrPatch,                  // plan rule, bay steps, bands, openings, roof, articulation, ornament
               motifs: [MotifId],                   // tiling families, glyph set, decal set
               props: PropSetId, furniture: FurnitureSetId,
               frequencies: { BuildingClass -> f32, PropClass -> f32 },
               permitted: { building classes, prop classes, materials },
               settlement: SettlementForm { compact|spread|linear|radial, courtyard, orientation },
               landmarks: [LandmarkKind],           // shrine, hall, gate, tower, beacon
               wear: WearProfile { phenomena per material class, age_scale, salvage: Option<CivilisationId> },
               brief: PathToDoc }
ClimateResponse { class, opening_ratio, wall_mass, roof_pitch: (f32, f32), courtyard, stilts, eave_depth,
                  orientation_deg, spread }
MaterialGraph { ops: [Op], params, palette_slots, resolution, wear_pass }
PropProgram  { cuboids: [Cuboid], attach: [...], reflect, repeat, leaves: [PartRef], material_slots }
DressingRule { surface_class, terms: [Term], examples: [RoomId], budget_ms }
```

The instance row gains `palette: u16, wear: u8` (to be checked against its 80 bytes); `District`
already carries `style` and `founded`; a `Settlement` record gains `civilisation` and its founding
cell.

**What to measure** (the demo pages, `docs/PROFILE.md`, the F1 overlay for every new pass):

- *variety:* unique modules and unique (module, variant, palette) triples visible per view; the
  longest repeated module sequence along any street (proposed in `city-generation.md`); the
  nearest distance between two visible instances of the same module and variant; ꟻLIP between the
  same street at two seeds and between two cultures' streets at one seed (both far from zero) — the
  owner's eye the final judge, captures at eye height in every set, TAA on;
- *memory:* texture bytes per style set and per civilisation at 1K and 2K, resident material pages
  during the flight, cluster pages per module set, against §6's estimates;
- *cook time:* per material graph, per style set's textures, per module set, cold and from the
  cache, one and six workers with equal digests where the output is deterministic (records, module
  geometry) and equal cache keys where it need not be (texels);
- *generation:* the response derivation per settlement, the grammar's derivation per building with
  the civilisation's patches, the dressing time per room in its `Low` job;
- *the frame:* no measurable change to city-blocks' 3.38 ms at 1440p from the palette and wear
  channels, the `glint` and `layered` classes costed in the resolve, the A/B harness and the mesh
  path against the fallback still at 0 pixels;
- *adaptivity:* the GDMC rubric of `procedural.md` §4 applied to a settlement moved between two
  atlas cells: does the same culture answer the new climate.

**Questions the owner must answer** (the plan assumes the answers in brackets):

1. Which eras first — medieval Mediterranean and contemporary downtown are set by #91 and
   city-blocks; is the futuristic set for #80's bases the third, or a primitive set for the island's
   villagers? [both, in steps 5–6, primitive first because it is cheaper]
2. How many cultures does the first showcase need, and are they Earth cultures, invented ones, or
   mixtures? [two real ones per era to calibrate against Oliver, then invented mixtures]
3. How much hand authoring is acceptable per culture: a brief and a reference board only, or also
   hero buildings, example rooms and a glyph set? [brief plus a dozen example rooms and one landmark
   per culture; no hand-modelled modules]
4. Do tents and organic forms need their non-grid path in the first year? [rectilinear huts first;
   yurts as revolved props]
5. Is a Substance seat worth its price for authoring the graphs against references, or is Material
   Maker plus Forge's contact-sheet tool enough? [Material Maker first]
6. For the futuristic look: the "used future" or the clean corporate one as the belt's default wear
   profile? [used, since the belt's rocks already are]
7. Should interiors of every era be enterable (#88), which sets the furniture-set cost per culture,
   or only the medieval and futuristic ones at first? [medieval and futuristic]

---

## What the numbers say

Vernacular is the ground: Oliver's estimate is "in excess of 90 per cent of the world's buildings",
"some 800 million dwellings", catalogued in 2 500 pages from 80 countries, and the bioclimatic
methods reduce a climate to four classes (Olgyay) or a dozen indicator counts (Mahoney) that output
opening percentages and wall-mass classes. Style grammars are small: Palladio's plans from one
parametric grammar, Wright's Prairie houses from a corpus of eleven, Malagueira's 1 200 houses from
a corpus of thirty-five, Islamic star patterns from a tiling and a contact angle, Gothic tracery from
"only a few basic geometric patterns", Puuc facades from four bands. Objects follow: Infinigen
Indoors ships 79 generators (17 furniture, 10 appliances at 112 parameters, 14 openings and stairs
at 127), ShapeAssembly writes chairs, tables and storage as cuboid programs, KitBash3D's free kit is
90 pieces, the City Sample's kits about 85 meshes each (`city-generation.md`). Materials: PatchMatch
made by-example synthesis "20-100x" faster, Wang tiles give non-periodic structure from eight to
sixteen tiles, phasor noise and glint BRDFs run in a pixel shader, and public procedural material
libraries hold "only a few thousand" graphs, which is the size of vocabulary to aim at. The games:
Age of Empires II reached eleven-plus architecture sets in twenty-five years, Age of Empires IV
builds four ages per civilisation, RimWorld sold five motif styles as a DLC, Minecraft ships five
biome village sets as data, No Man's Sky's races are six archetypes at seven-to-three ratios, Star
Citizen has about ten manufacturers, Starfield three hand-built cities over a thousand planets.
Forge's estimates, to be replaced by the tools' printed numbers: 100–150 MiB of textures and
100–150 MiB of cluster pages per style set, 10–20 s to cook a 300-module set at today's rate,
seconds to a minute for a material set's textures, microseconds for a climate response, and two
extra bytes in the instance row.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **A grammar for timber framing or half-timbered facades** — none found; two searches returned
  only historical pages and the general facade-grammar papers (Müller 2007, layer-based facades,
  Pro-DG 2025). Timber framing is treated as a band-and-bay style (posts, rails and braces as trim
  rows and modules over an infill tileable) without a dedicated source.
- **A published procedural castle or fortification grammar** — only Blender add-ons, a Minecraft
  mod post and a 2021 arXiv paper on volumetric procedural models with a castle-gate example;
  castles are left to the kit-plus-skeleton route (`city-generation.md`'s Kelly & Wonka).
- **A Firaxis talk on Civilization VI's architecture sets** — the GDC 2017 talk found is a design
  retrospective; the art director's interviews are press pieces. The entry rests on a fan wiki and
  is graded weak.
- **A Bethesda talk on Starfield's cities or kits** — the only GDC 2024 Starfield item found is a
  quest-design retrospective; the entry rests on press coverage and is graded weak.
- **Primary Star Citizen, Mass Effect and Kenshi design sources** — fan wikis stand for all three;
  graded weak.
- **The Infinigen repository's generator directory** — the README was read on GitHub, but tree and
  blob pages and the API returned 404/403 through the proxy, so the generator counts come from the
  paper as extracted by the search engine, not from the file listing; the CVPR and arXiv hosts are
  blocked, so the quoted sentences are the search engine's extracts of the abstract.
- **A "used future" primary source** (Rinzler's *The Making of Star Wars*, a Ron Cobb interview) —
  the searches returned obituaries and fan pages; the phrase is used descriptively and the greeble
  entry carries the practice through its Wikipedia page.
- **Machine-learned material and layout generation as a dependency** (MaterialGAN, MatFormer,
  ATISS, diffusion floor plans, Pro-DG) — cited as reference only (P3/P8, D-016, editability).
  Listed so nobody re-derives the omission.
- **Kelly & Wonka, Wonka 2003, Müller 2006 (Pompeii), CGA++, Bethesda's kit talks, the City Sample,
  WFC and Townscaper** — in `city-generation.md`; **Perlin, Worley, Ebert et al., Cook & DeRose,
  Lagae, Heitz–Neyret, Deliot–Heitz, Burley, Mikkelsen, Wronski, Quilez's SDFs, MATch, Hu 2022,
  Green's SDF glyphs** — in `procedural.md` §5; **trim sheets, texture bombing, CC0 libraries, BC
  formats and KTX2, surface gradients, the unified material row** — in `vegetation-materials.md`;
  **Lagarde's wet surfaces, Köppen and the atlas** — in `planet-environment.md`. Referenced, not
  repeated.
- **Fathy's *Architecture for the Poor* (1973)** — the 1986 book was preferred as the technical
  one; the 1973 book was not separately verified.
- **Vehicle and signage generation papers** — none looked for beyond the entries above; §3's design
  for them is derived, not cited.

---

## Verification notes

Checked on 2026-09-26 with WebSearch and WebFetch only; no browser pane and no video pages. The
session's egress proxy served `github.com` to WebFetch and refused every other host tried
(arxiv.org and openaccess.thecvf.com to WebFetch; through `curl`, even github.com, dl.acm.org,
en.wikipedia.org, gdcvault.com, iquilezles.org and the search engines' own pages answered a 403 on
CONNECT). GitHub tree and blob pages and the API returned 404/403; only repository front pages
(READMEs) were readable, and the GitHub MCP tool is scoped to this repository. Verification
therefore has two grades; every entry not in the first carries "(verified through search results)"
on its URL line.

- **Fetched and read (GitHub READMEs):** princeton-vl/infinigen (BSD-3-Clause; the transpiler
  sentence is verbatim); RodZill4/material-maker (MIT; the two description sentences are verbatim);
  mfx-inria/phasornoise (AGPL-3.0; Python and OpenCL contents); rkjones4/ShapeAssembly (the
  "Executing a ShapeAssembly program…" and datasets sentences are verbatim); misode/mcmeta (the
  description sentence is verbatim).
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  volume, pages, DOI or ISBN, and the sentences quoted, which are the search engine's extracts of
  the page named): every book through its publisher's page, Open Library, WorldCat or Wikipedia
  (Rapoport, Oliver ×2, Fathy, Olgyay, Givoni, Koenigsberger, DeKay & Brown, Alexander, Ching,
  Banham, NASA SP-413 via NSS and ADS, O'Neill, Dorsey–Rushmeier–Sillion via the Elsevier
  front-matter listing); every journal and conference paper through its publisher's page (SAGE
  and RePEc for the five *Environment and Planning B* grammars, ACM DL, Wiley, Springer,
  ScienceDirect, the Eurographics digital library, IEEE Xplore) and, where one exists, an author's
  or an institution's PDF listing (the CMU course copies of Stiny & Mitchell, Flemming and Knight,
  MIT DSpace for Li, Waterloo and Washington for Kaplan, Calgary for Hamekasi, Berkeley for Efros &
  Leung and Merrell, Princeton for PatchMatch, Stanford for Fisher, TU Delft for Tutenel, Yale for
  Dorsey and Hu, UCSD and MIT CSAIL for weathered stone, Microsoft Research for appearance
  manifolds, HAL for Tricard and Mérillou, NIST and Loughborough for Buswell) or the SIGGRAPH
  history archive (Turk, Witkin & Kass, Kwatra, Barnes, Cohen, Chen, Wang, MaterialGAN, MatFormer,
  Dorsey 1996 ×2); Infinigen Indoors through the CVPR open-access listing, Princeton, alphaXiv and
  the arXiv HTML listing (the generator counts); ShapeAssembly through its project page and arXiv
  listing; Schumacher through his own pages and Architects' Journal; the Greeble page through
  Wikipedia with the VFX-archaeology and Den of Geek pieces for the Burton quote; Quilez, Blender,
  Houdini and Substance Designer through their sites' listings; the GDC talks through GDC Vault
  listings 1024310, 1023251, 1023158 and 1021805 (Duncan's also through Game Developer, the Hello
  Games post and the Internet Archive copy); KitBash3D through its product page and CG Channel;
  the games through the wiki pages named, the PlayStation Blog post, the Durand interviews (Game
  Developer, TheSixthAxis, History Respawned) and the Starfield press pieces (PCGamesN, GamesRadar,
  TechRadar).
- **Weaker confirmations, stated plainly.** Rapoport's and Fathy's sentences are catalogue
  descriptions as extracted, not the books' text. The Mahoney recommendations are paraphrased from
  the method's description, not quoted from the tables. The grammar papers' sentences are the
  search engine's extracts of their abstracts; Knight's paper is described from the CMU course
  framing. The Kaplan 2005 construction is summarised. The greeble history is Wikipedia's and Den of
  Geek's; the NASA figures are Wikipedia's rendering of SP-413. The Infinigen Indoors generator
  counts are from third-party summaries of the paper. The Age of Empires II set count is the wiki's
  wording at one point and moves with expansions; the RimWorld, Kenshi, Horizon, Mass Effect and
  Star Citizen sentences are the wikis' and the blog's; the Starfield sentences are press summaries.
  Page numbers given without a DOI in the search record (Greuter 2003, Kwatra 2003, Dorsey &
  Hanrahan 1996) were left out of the entries rather than remembered.
- **Forge's own numbers** (the 8 M-triangle ground cooked in 12 s, 25.8 M triangles in 612 k
  clusters and 983 MiB of pages, the 80-byte instance, 3.38 ms at 1440p, 128 KiB pages, hex-tiling
  in #66, the regional material set's counts) are from `docs/demos/city-blocks.md`,
  `docs/research/city-generation.md` and `docs/research/vegetation-materials.md` as of 2026-09-26.
- **Numbers to re-check before they enter a spec:** every figure in §6's cost paragraph and in the
  recommendation (texture bytes per set, module counts per era, cook seconds, the two spare bytes in
  the instance row) is an estimate, marked as such, to be replaced by the tools' printed numbers;
  the module counts per era are proposals for the owner; the 1.8 m bay as 3.6 steps is a test case
  for the grid rule, not a decision.
