module m;
  int x;
  initial randsequence (main)
    main : first second := 2 | rand join (0.5) a b | third;
    first : if (x > 1) a else b;
    second : case (x) 0, 1 : a; default : b; endcase;
    third : repeat (3) a { x++; };
    void add(int y = 1) : { x += y; if (x > 9) break; else return; };
    a : add(2);
    b : add;
  endsequence
endmodule
