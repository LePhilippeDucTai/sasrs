/* J02-P6 - contrat des procedures graphiques : plus aucun repli silencieux.
   ODS GRAPHICS reste desactive : aucune image n'est produite, le log est donc
   identique avec et sans --features graphics. Chaque diagnostic est fige par
   un test ra_j02_p6_* (src/procs/{sgplot,gplot,gchart,plot}/contract_tests.rs,
   src/ods_graphics/contract_tests.rs). */
data xy;
  input x y z g $;
  datalines;
1 2 2 a
2 4 10 b
3 3 2 a
;
run;

/* Options d'affichage non rendues : un WARNING par option ; l'etape tourne. */
proc sgplot data=xy noautolegend;
  scatter x=x y=y / group=g markerattrs=(symbol=circle);
  vbar g / response=y stat=sum;
  xaxis label='X' values=(0 to 10 by 2) grid;
run;

/* BY : ERROR, etape rejetee (images par groupe BY : J13-P4). */
proc sgplot data=xy;
  by g;
  scatter x=x y=y;
run;

/* Table et variables validees dans les deux builds. */
proc sgplot data=xy;
  scatter x=x y=nope;
run;

/* Instruction de trace non implementee : message du catalogue du contrat. */
proc sgplot data=xy;
  hbox y;
run;

proc gplot data=xy;
  symbol1 i=join h=2;
  axis1 order=(0 to 10 by 2) label=(a=90 'X');
  plot y*x / haxis=axis1 overlay;
run;

proc gchart data=xy;
  vbar g / type=percent subgroup=g;
  hbar g;
run;

proc plot data=xy;
  plot y*x='*' / box;
run;

/* RESET=WIDTH remet la largeur par defaut ; l'index d'image ne peut pas etre
   remis a zero (WARNING). */
ods graphics / width=1000;
ods graphics / reset=width;
ods graphics / reset=index;
