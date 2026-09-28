/* PROC TRANSPOSE — ID multi-variables + DELIMITER=, LET, IDLABEL, LABEL=.
   Cf. SAS 9.4 Procedures Guide, TRANSPOSE Procedure : le nom des variables
   transposees est la concatenation des valeurs formatees des variables ID,
   DELIMITER= etant insere entre elles ; sans LET, des valeurs ID dupliquees
   arretent la procedure avec une erreur, avec LET « the procedure issues a
   warning message and continues processing, transposing the observation
   containing the last occurrence of the duplicate » ; IDLABEL fournit le
   label des variables transposees et LABEL= renomme la variable qui les
   contient (defaut _LABEL_). */
libname ind 'data';

proc transpose data=ind.u out=out let delimiter=_ label=mlabel;
  id grp metric;
  idlabel lbl;
  var value;
run;
