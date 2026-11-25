module t;
  typedef struct { int a; logic [3:0] c; } s_t;
  s_t s;
  int n[2][4];
  initial begin
    s = '{int: 5, logic [3:0]: 4'h3};
    s = '{a: 1, default: 0};
    n = '{2{'{2{1, 2}}}};
  end
endmodule
