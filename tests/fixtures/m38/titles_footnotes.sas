/* M38.1 — TITLE1..TITLE9 / FOOTNOTE1..FOOTNOTE9 : titres multiples.
   Oracle : chaque niveau de titre actif est rendu centre, dans l'ordre des
   niveaux, en tete de la sortie de chaque proc ; les footnotes actives sont
   rendues centrees en pied, apres le contenu du proc. Semantique
   d'effacement SAS : `TITLEn 'x'` pose le niveau n ET efface les niveaux
   n+1..9 ; `title;` seul efface tout (retour au titre par defaut
   "The SAS System") ; idem pour FOOTNOTE. Gotcha SAS verrouille aussi :
   les footnotes sont ecrites quand la page se TERMINE (au demarrage du proc
   suivant ou en fin de programme), donc un statement FOOTNOTE place entre
   deux procs s'applique a la page encore ouverte du proc precedent. */

libname d 'data';

/* 1) Trois niveaux de titre + deux footnotes : les trois titres centres
   au-dessus du PRINT, les deux footnotes centrees en dessous. */
title1 'Etude demographique';
title2 'Cohorte CLASS';
title3 'Brouillon v1';
footnote1 'Source : d.class';
footnote2 'Confidentiel';

proc print data=d.class;
    var name sex age;
run;

/* 2) Redefinir TITLE2 efface TITLE3 (niveaux superieurs a n effaces) :
   seuls TITLE1 et le nouveau TITLE2 coiffent le MEANS. Les footnotes,
   inchangees, restent actives. */
title2 'Cohorte CLASS - version revue';

proc means data=d.class;
    var age;
run;

/* 3) `title;` efface tous les titres et `footnote;` toutes les footnotes :
   le dernier PRINT retombe sur "The SAS System", sans pied de page.
   Gotcha : la page du MEANS ci-dessus n'etant terminee qu'ici, le
   `footnote;` la prive AUSSI de ses footnotes (comportement SAS). */
title;
footnote;

proc print data=d.class;
    var name age;
run;
