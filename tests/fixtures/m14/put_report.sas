/* M14.2 : FILE LOG + PUT (sortie vers le log et construction de lignes). */

* PUT vers le LOG : valeurs numériques et chaînes. ;
data _null_;
  x = 3.14159;
  name = 'Claude';
  put 'Valeur pi = ' x 8.4;
  put 'Nom       = ' name $;
run;

* PUT avec hold (@) pour construire une ligne multi-items. ;
data _null_;
  do i = 1 to 5;
    put i @;
  end;
  put;
run;

* PUT _all_ : toutes les variables de l'étape. ;
data _null_;
  a = 1; b = 2; c = 'abc';
  put _all_;
run;

* PUT FILE PRINT : vers le listing. ;
data _null_;
  file print;
  put 'Titre du rapport via PUT FILE PRINT';
  put '------------------------------------';
  do i = 1 to 3;
    put 'Ligne ' i;
  end;
run;
