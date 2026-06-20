/* M33.1 : PROC FREQ — options differees WEIGHT / BY / TABLES .../LIST et
   tables n-voies (>=3). Donnees : sashelp.class (name sex age height weight).

   On derive agegrp = Old si age>=14 sinon Yng, et un poids entier wt = 2
   (chaque eleve compte double) pour rendre l'effet WEIGHT lisible : toutes les
   frequences ponderees valent le DOUBLE du comptage simple.

   - WEIGHT wt : table 1-voie de sex ponderee -> F 18, M 20, total 38
     (comptage simple F 9 / M 10 ; *2). Percent F 47.37 / M 52.63.
   - BY sex : une analyse FREQ independante par sexe (table de agegrp).
   - tables sex*agegrp / list : rendu LIST (une ligne par cellule non vide).
   - tables sex*agegrp*... pas applique ici ; on couvre le n-voies par
     sex*agegrp*age (stratifie par sex) plus bas. */
libname d 'data';

data class;
  set d.class;
  if age >= 14 then agegrp = 'Old';
  else agegrp = 'Yng';
  wt = 2;
run;

proc sort data=class out=class_s;
  by sex;
run;

title 'Weighted one-way frequency of sex (wt=2)';
proc freq data=class;
  weight wt;
  tables sex;
run;

title 'Frequency of age-group BY sex';
proc freq data=class_s;
  by sex;
  tables agegrp;
run;

title 'Crosstab sex by age-group in LIST layout';
proc freq data=class;
  tables sex*agegrp / list;
run;

title 'Three-way table sex by agegrp by age (stratified)';
proc freq data=class;
  tables sex*agegrp*age;
run;
