/* M18 : formats & informats — formats étendus (M18.1), INVALUE (M18.2),    */
/* PICTURE (M18.3).                                                          */

/* M18.2 — INVALUE : informats utilisateur (résultat numérique) */
proc format;
  invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0 other=.;
  /* M18.3 — PICTURE : formats image */
  picture dollarpic low-high = '000,009.99' (prefix='$');
run;

data work.test;
  /* M18.2 — application via la fonction INPUT() */
  g_a = input('A', grade.);    /* 4 */
  g_c = input('C', grade.);    /* 2 */
  g_x = input('X', grade.);    /* . (other) */

  /* M18.1 — formats numériques étendus via PUT() */
  roman_9 = put(9, roman.);          /* IX */
  comma_x = put(12345.6, comma10.2); /* 12,345.60 */

  /* M18.1 — formats date étendus (21915 = 2020-01-01, un mercredi) */
  dow = put(21915, downame.);   /* Wednesday */
  mon = put(21915, monname.);   /* January */
  qtr = put(21915, qtr.);       /* 1 */

  /* M18.3 — application du PICTURE via PUT() */
  dp = put(1234.5, dollarpic.); /* $1,234.50 */

  output;
run;

title 'M18 Formats & Informats';
proc print data=work.test;
run;
