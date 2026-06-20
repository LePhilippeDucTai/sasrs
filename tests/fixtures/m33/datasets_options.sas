/* M33.8 : PROC DATASETS — operations differees COPY / EXCHANGE / SAVE / MODIFY.
   On cree quelques petites tables WORK puis on exerce chaque operation et on
   verrouille le resultat via des PROC PRINT et le listing du repertoire.

   Tables initiales : ONE (x=1,2), TWO (x=9), THREE (x=7).

   1) COPY out=work in=d; select class : copie d.class (sashelp.class) dans WORK.
   2) EXCHANGE two=three : echange les noms TWO et THREE.
      -> apres : TWO contient l'ancien THREE (7), THREE contient l'ancien TWO (9).
   3) MODIFY one; rename x=y; label y='renamed' : renomme la variable x en y.
   4) SAVE one two class : supprime toutes les tables de WORK sauf ONE/TWO/CLASS
      (THREE est donc supprimee) ; le listing final montre le repertoire. */
libname d 'data';

data one;   x = 1; output; x = 2; output; run;
data two;   x = 9; output; run;
data three; x = 7; output; run;

title 'DATASETS: COPY d.class into WORK, EXCHANGE two/three, MODIFY one';
proc datasets lib=work nolist;
  copy out=work in=d;
  select class;
  exchange two=three;
  modify one;
    rename x=y;
    label y='renamed';
quit;

title 'WORK.ONE after MODIFY (x renamed to y)';
proc print data=one;
run;

title 'WORK.TWO after EXCHANGE (now holds old THREE = 7)';
proc print data=two;
run;

title 'WORK.THREE after EXCHANGE (now holds old TWO = 9)';
proc print data=three;
run;

title 'DATASETS: SAVE one two class (THREE deleted), then directory listing';
proc datasets lib=work;
  save one two class;
quit;
