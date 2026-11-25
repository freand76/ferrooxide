// expected
// Replication in assignment patterns (IEEE 1800-2017 10.10.1: `'{n{...}}`, also nested and with
// a parameter count) and type keys (10.9.2: `int: v` sets the members of that type; member
// names win over type keys, type keys over default).
module t;
  localparam N = 3;
  typedef struct { int a; int b; logic [3:0] c; } s_t;
  struct { int X, Y, Z; } xyz = '{N{7}};
  int n[1:2][1:6] = '{2{'{3{4, 5}}}};
  s_t s;
  int q[$];
  initial begin
    $display("%0d %0d %0d", xyz.X, xyz.Y, xyz.Z);
    $display("%0d %0d %0d %0d", n[1][1], n[1][2], n[2][5], n[2][6]);
    s = '{int: 5, default: 4'h9};
    $display("%0d %0d %h", s.a, s.b, s.c);
    s = '{b: 1, int: 2, logic [3:0]: 4'h3};
    $display("%0d %0d %h", s.a, s.b, s.c);
    q = '{2{1, 2}};
    $display("%p", q);
  end
endmodule
