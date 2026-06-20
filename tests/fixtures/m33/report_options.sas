/* M33.5 : PROC REPORT — options DEFINE differees (FORMAT= / WIDTH= / SPACING=)
   et COMPUTE avec reference positionnelle _Cn_ + LINE formatee.
   Donnees : sashelp.class (name sex age height weight). Tout est verifiable
   a la main.

   Sommaire groupe par SEX, colonnes :
     1 = sex   (GROUP)
     2 = height (ANALYSIS MEAN, FORMAT=6.2)
     3 = weight (ANALYSIS MEAN, WIDTH=10 SPACING=5)
     4 = ratio  (COMPUTED = _C3_ / _C2_, FORMAT=6.3)

   Oracles (means sur les valeurs non manquantes du groupe) :
     F (n=9) : meanH = 545.3/9 = 60.588889  -> 6.2  -> "60.59"
               meanW = 810.0/9 = 90.111111
               ratio = meanW/meanH = 1.487255 -> 6.3 -> "1.487"
     M (n=10): meanH = 639.1/10 = 63.91      -> 6.2  -> "63.91"
               meanW = 1089.5/10 = 108.95
               ratio = 108.95/63.91 = 1.704741 -> 6.3 -> "1.705"

   WIDTH=10 elargit la colonne weight ; SPACING=5 met 5 espaces avant.
   La ligne COMPUTE AFTER imprime le total general des hauteurs (somme des
   moyennes affichees n'a pas de sens : on imprime le grand total RBREAK des
   moyennes via _C2_ formate). */
libname d 'data';

title 'REPORT: FORMAT= / WIDTH= / SPACING= + computed _Cn_ ratio';
proc report data=d.class nowd;
  column sex height weight ratio;
  define sex    / group;
  define height / analysis mean format=6.2;
  define weight / analysis mean width=10 spacing=5;
  define ratio  / computed format=6.3;
  compute ratio;
    ratio = _c3_ / _c2_;
  endcomp;
run;
