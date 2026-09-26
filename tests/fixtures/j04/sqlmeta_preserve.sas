/* J04-P4 : metadonnees apres CREATE TABLE AS SELECT — une colonne reprise
   telle quelle conserve format, label et longueur de sa source (sidecar) ;
   une colonne calculee n'herite de rien. */
libname d 'data';

/* Source portant format/label/longueur (persistes dans le sidecar J04-P1). */
data work.src;
  length name $ 12;
  set d.class(keep=name sex age height);
  format height 8.2 name $char12.;
  label name='Full name' height='Height (in)';
run;

title 'Source metadata';
proc contents data=work.src;
run;

/* Colonnes reprises telles quelles (nue, renommee via AS) : conservees. */
proc sql;
  create table work.kept as
  select name, height as h, age
  from work.src;
quit;

title 'CREATE TABLE AS SELECT — columns taken as-is keep metadata';
proc contents data=work.kept;
run;

/* Colonne calculee : aucune metadonnee heritee. */
proc sql;
  create table work.calc as
  select name, height / age as ratio
  from work.src;
quit;

title 'CREATE TABLE AS SELECT — calculated column has no metadata';
proc contents data=work.calc;
run;

/* DELETE FROM reecrit la table sans sucer les metadonnees. */
proc sql;
  delete from work.src where age > 15;
quit;

title 'After DELETE FROM — metadata survives the rewrite';
proc contents data=work.src;
run;
