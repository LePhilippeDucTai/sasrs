/* M33.9 : PROC SORT options deferred (TAGSORT, SORTSEQ=, KEY=)
           + PROC APPEND options deferred (NOWARN, APPENDVER=).
   Donnees : sashelp.class (name sex age height weight), n=19.

   Oracles (verifies a la main) :
   1. TAGSORT + SORTSEQ=ASCII : identical to a plain sort on age ascending.
      Ages in class ascending: 11,11,12,12,12,12,12,13,13,13,14,14,14,14,15,15,15,15,16
      (19 obs ; Joyce age=11, Thomas age=11 premiers).
   2. KEY=age / descending : age trie du plus grand au plus petit.
      Premiere ligne age=16 (Philip), dernieres age=11 (Joyce, Thomas).
   3. PROC APPEND FORCE NOWARN : appender un dataset avec variable en plus
      (z absente de BASE). Sans NOWARN, un WARNING serait emis. Avec NOWARN,
      le log ne contient pas de WARNING "not found on BASE".
      Resultat : 4 observations (2 base + 2 data), variable x seulement.
   4. APPENDVER=V6 : identique a un append ordinaire (no-op hint). */
libname d 'data';

/* ==== 1. TAGSORT + SORTSEQ=ASCII : sort age ascending (hint seulement) ==== */
title 'SORT with TAGSORT and SORTSEQ=ASCII (age ascending)';
proc sort data=d.class out=class_sorted tagsort sortseq=ascii;
  by age;
run;
proc print data=class_sorted noobs;
  var name age;
run;

/* ==== 2. KEY=age / descending : age du plus grand au plus petit ==== */
title 'SORT with KEY=age / descending';
proc sort data=d.class out=class_keydesc;
  key=age / descending;
run;
proc print data=class_keydesc noobs;
  var name age;
run;

/* ==== 3. PROC APPEND FORCE NOWARN : suppression du WARNING ==== */
/* Construire BASE avec seulement x. */
data base_ds;
  x = 1; output;
  x = 2; output;
run;
/* DATA a x et z (z absente de BASE = aurait ete un WARNING sans NOWARN). */
data data_ds;
  x = 3; z = 99; output;
  x = 4; z = 88; output;
run;

title 'APPEND FORCE NOWARN (no warning expected)';
proc append base=base_ds data=data_ds force nowarn;
run;
proc print data=base_ds noobs;
run;

/* ==== 4. APPENDVER=V6 no-op : resultat identique a un append ordinaire ==== */
data base2;
  x = 10; output;
run;
data data2;
  x = 20; output;
run;

title 'APPEND with APPENDVER=V6 (no-op hint)';
proc append base=base2 data=data2 appendver=v6;
run;
proc print data=base2 noobs;
run;
