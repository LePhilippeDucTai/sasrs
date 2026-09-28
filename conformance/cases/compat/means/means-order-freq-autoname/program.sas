/* PROC MEANS — ORDER=FREQ, MAXDEC=, ID, FREQ, OUTPUT / AUTONAME.
   Cf. SAS 9.4 Procedures Guide, MEANS Procedure : ORDER=FREQ ordonne les
   niveaux par effectif décroissant ; FREQ pondère N et la variance par la
   somme des poids ; ID copie la variable ID dans l'OUT= ; AUTONAME nomme
   les statistiques <variable>_<statistique> ; MAXDEC= ne touche que le
   rapport imprimé, pas l'OUT=. */
libname ind 'data';

proc means data=ind.weights nway noprint order=freq maxdec=2;
  class group;
  var value;
  freq w;
  id price;
  output out=out mean= std= / autoname;
run;
