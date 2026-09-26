/* J04-P4 : FORMAT= / LABEL= / LENGTH= dans le select-list — seuls moyens
   d'attacher des metadonnees a une colonne (calculee ou reprise). LENGTH=
   tronque reellement les valeurs caracteres. */
libname d 'data';

data work.src;
  length name $ 12;
  set d.class(keep=name age height);
  label name='Full name';
run;

/* Attributs sur une colonne reprise et sur des colonnes calculees. */
proc sql;
  create table work.attr as
  select name label='Court' length=3 format=$3.,
         height / age as ratio label='Ratio h/a' format=8.3,
         age + 100 as older format=best5.
  from work.src;
quit;

title 'FORMAT=/LABEL=/LENGTH= in the SELECT list';
proc print data=work.attr;
run;

title 'Metadata';
proc contents data=work.attr;
run;
