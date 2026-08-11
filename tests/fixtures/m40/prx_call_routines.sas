/* M40.1 - Perl regular expressions (PRX).
   1) PRXMATCH avec pattern litteral (position 1-based, 0 si pas de match).
   2) PRXCHANGE fonction : substitution s/../../ avec backreferences $1/$2,
      times=-1 (toutes les occurrences).
   3) CALL PRXSUBSTR + PRXPOSN : position/longueur du match et des groupes.
   4) CALL PRXNEXT en boucle : extraction de tous les matches successifs
      (start avance a position + max(length,1) apres chaque match).
   5) CALL PRXCHANGE avec resultat, longueur stockee et nombre de
      remplacements. */

data work.match;
  input text $20.;
  pos_cat = prxmatch('/cat/', text);
  pos_ci  = prxmatch('/CAT/i', text);
  datalines;
concatenate
dog
Catalog
;
run;

title "PRXMATCH : position du motif (sensible et insensible a la casse)";
proc print data=work.match noobs;
run;
title;

data work.swap;
  length swapped $20;
  input name $20.;
  /* Prenom Nom -> Nom, Prenom */
  swapped = prxchange('s/(\w+) (\w+)/$2, $1/', -1, name);
  datalines;
John Smith
Jane Doe
;
run;

title "PRXCHANGE : inversion Prenom Nom -> Nom, Prenom";
proc print data=work.swap noobs;
run;
title;

data work.phone;
  length area $3 exch $4;
  input text $30.;
  re = prxparse('/(\d{3})-(\d{4})/');
  call prxsubstr(re, text, pos, len);
  if pos > 0 then do;
    call prxposn(re, 1, p1, l1);
    area = substr(text, p1, l1);
    call prxposn(re, 2, p2, l2);
    exch = substr(text, p2, l2);
  end;
  drop re p1 l1 p2 l2;
  datalines;
call 555-1234 now
no phone here
;
run;

title "CALL PRXSUBSTR + CALL PRXPOSN : match et groupes captures";
proc print data=work.phone noobs;
run;
title;

data work.words;
  length word $10;
  text = 'one two three';
  re = prxparse('/\w+/');
  start = 1;
  lim = length(text);
  call prxnext(re, start, lim, text, pos, len);
  do while (pos > 0);
    word = substr(text, pos, len);
    output;
    call prxnext(re, start, lim, text, pos, len);
  end;
  keep word pos len;
run;

title "CALL PRXNEXT : tous les mots extraits en boucle";
proc print data=work.words noobs;
run;
title;

data work.redact;
  length clean $30;
  text = 'card 1234 and 5678 end';
  re = prxparse('s/\d+/XXXX/');
  call prxchange(re, -1, text, clean, rlen, trunc, nchg);
  keep clean rlen trunc nchg;
run;

title "CALL PRXCHANGE : masquage des nombres + compteurs";
proc print data=work.redact noobs;
run;
title;
